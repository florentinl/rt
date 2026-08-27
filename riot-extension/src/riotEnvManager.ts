/**
 * Riot Environment Manager
 * Coordinates environment discovery, management, and lifecycle.
 */

import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  DidChangeEnvironmentEventArgs,
  DidChangeEnvironmentsEventArgs,
  EnvironmentChangeKind,
  EnvironmentManager,
  GetEnvironmentScope,
  GetEnvironmentsScope,
  IconPath,
  PythonCommandRunConfiguration,
  PythonEnvironment,
  RefreshEnvironmentsScope,
  ResolveEnvironmentContext,
  SetEnvironmentScope,
} from "./api";
import { RtCliService } from "./services/rtCliService";
import { TestingConfigurationManager } from "./services/testingConfigManager";
import { WorkspaceResolver } from "./services/workspaceResolver";
import { buildManagerId, RtExecutionContext, RtVenv } from "./types/rtTypes";

export { RtExecutionContext, RtVenv } from "./types/rtTypes";

export interface RiotEnvironmentRecord {
  readonly workspaceRoot: string;
  readonly venv: RtVenv;
  readonly context: RtExecutionContext;
  readonly environment: PythonEnvironment;
}

interface WorkspaceSnapshot {
  readonly venvs: RtVenv[];
  readonly environments: PythonEnvironment[];
  readonly recordsById: Map<string, RiotEnvironmentRecord>;
  readonly recordsByHash: Map<string, RiotEnvironmentRecord>;
}

/** Main environment manager implementation. */
export class RiotEnvManager implements EnvironmentManager {
  readonly name = "riot";
  readonly displayName = "Riot";
  readonly description = "Immutable environments declared in riotfile.py";
  readonly tooltip = "Riot virtual environments";
  readonly iconPath?: IconPath;
  readonly log: vscode.LogOutputChannel;
  readonly managerId: string;
  readonly preferredPackageManagerId: string;

  private readonly cliService: RtCliService;
  private readonly workspaceResolver = new WorkspaceResolver();
  private readonly testingConfigManager: TestingConfigurationManager;
  private readonly currentEnvironment = new Map<string, PythonEnvironment>();
  private readonly snapshots = new Map<string, WorkspaceSnapshot>();
  private readonly ongoingLoads = new Map<string, Promise<WorkspaceSnapshot>>();
  private readonly ongoingActivations = new Map<string, AbortController>();

  private readonly envsEmitter =
    new vscode.EventEmitter<DidChangeEnvironmentsEventArgs>();
  private readonly envEmitter =
    new vscode.EventEmitter<DidChangeEnvironmentEventArgs>();

  readonly onDidChangeEnvironments = this.envsEmitter.event;
  readonly onDidChangeEnvironment = this.envEmitter.event;

  constructor(
    log: vscode.LogOutputChannel,
    extensionId: string,
    extensionPath?: string,
  ) {
    this.managerId = buildManagerId(extensionId, this.name);
    this.preferredPackageManagerId = buildManagerId(extensionId, "rt");
    this.log = log;
    this.cliService = new RtCliService(log, extensionPath);
    this.testingConfigManager = new TestingConfigurationManager(log);
  }

  async refresh(scope: RefreshEnvironmentsScope): Promise<void> {
    const folders = this.workspaceResolver.getWorkspaceFoldersForScope(scope);
    const changes: DidChangeEnvironmentsEventArgs = [];

    await Promise.all(
      folders.map(async (folder) => {
        const cwd = folder.fsPath;
        const previous = this.snapshots.get(cwd)?.environments ?? [];
        const nextSnapshot = await this.loadWorkspace(cwd, true);
        changes.push(...this.computeChanges(previous, nextSnapshot.environments));
        await this.reconcileCurrentEnvironment(folder, nextSnapshot);
      }),
    );

    if (changes.length > 0) {
      this.envsEmitter.fire(changes);
    }
  }

  async getEnvironments(
    scope: GetEnvironmentsScope,
  ): Promise<PythonEnvironment[]> {
    if (scope === "global") {
      return [];
    }

    const folders = this.workspaceResolver.getWorkspaceFoldersForScope(scope);
    const snapshots = await Promise.all(
      folders.map((folder) => this.loadWorkspace(folder.fsPath, false)),
    );
    return snapshots.flatMap((snapshot) => snapshot.environments);
  }

  async set(
    scope: SetEnvironmentScope,
    environment?: PythonEnvironment,
  ): Promise<void> {
    const folders = this.workspaceResolver.getWorkspaceFoldersForSetScope(
      scope,
      environment,
    );
    if (folders.length === 0) {
      if (environment) {
        void vscode.window.showErrorMessage(
          "Unable to determine workspace folder. Open a file in the target workspace and try again.",
        );
      }
      return;
    }

    for (const folder of folders) {
      const cwd = folder.fsPath;
      const previous = this.currentEnvironment.get(cwd);
      this.cancelActivation(cwd);

      if (environment) {
        await this.activateEnvironment(cwd, folder, environment, previous);
      } else {
        await this.clearEnvironment(cwd, folder, previous);
      }
    }
  }

  async get(
    scope: GetEnvironmentScope,
  ): Promise<PythonEnvironment | undefined> {
    const folder = this.workspaceResolver.getWorkspaceFolder(scope);
    if (!folder) {
      return undefined;
    }

    const current = this.currentEnvironment.get(folder.fsPath);
    if (current && this.environmentExecutableExists(current)) {
      return current;
    }

    this.currentEnvironment.delete(folder.fsPath);
    return undefined;
  }

  async resolve(
    context: ResolveEnvironmentContext,
  ): Promise<PythonEnvironment | undefined> {
    const scopedFolder = this.workspaceResolver.getWorkspaceFolder(context);
    const allFolders = this.workspaceResolver.getWorkspaceFolders();
    const folders = scopedFolder
      ? [
          scopedFolder,
          ...allFolders.filter(
            (folder) => folder.toString() !== scopedFolder.toString(),
          ),
        ]
      : allFolders;

    const snapshots = await Promise.all(
      folders.map((folder) => this.loadWorkspace(folder.fsPath, false)),
    );
    for (const snapshot of snapshots) {
      const match = snapshot.environments.find((environment) =>
        this.matchesPath(environment, context.fsPath),
      );
      if (match) {
        return match;
      }
    }
    return undefined;
  }

  async clearCache(): Promise<void> {
    this.snapshots.clear();
  }

  /** Force reinstall an environment while preserving its selection. */
  async forceReinstallEnvironment(
    scope: GetEnvironmentScope,
    environment: PythonEnvironment,
  ): Promise<void> {
    const folder = this.workspaceResolver.getWorkspaceFolder(scope, environment);
    if (!folder) {
      throw new Error("Unable to determine the Riot workspace for this environment");
    }

    const record = await this.getEnvironmentRecord(environment, false);
    if (!record) {
      throw new Error(`Riot environment not found: ${environment.displayName}`);
    }

    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: `Rebuilding ${environment.displayName}`,
        cancellable: true,
      },
      async (progress, token) => {
        const controller = new AbortController();
        token.onCancellationRequested(() => controller.abort());
        progress.report({ message: "Running rt build --force-reinstall" });
        await this.cliService.buildEnvironment(
          record.context.hash,
          folder.fsPath,
          {
            forceReinstall: true,
            signal: controller.signal,
          },
        );
      },
    );

    const refreshed = await this.loadWorkspace(folder.fsPath, true);
    const updated = this.findRecord(refreshed, environment)?.environment;
    if (updated && this.currentEnvironment.has(folder.fsPath)) {
      this.currentEnvironment.set(folder.fsPath, updated);
    }
  }

  /** Return cached/indexed Riot data for the package provider. */
  async getEnvironmentRecord(
    environment: PythonEnvironment,
    forceRefresh: boolean,
  ): Promise<RiotEnvironmentRecord | undefined> {
    const known = this.findKnownRecord(environment);
    const folder = known
      ? vscode.Uri.file(known.workspaceRoot)
      : this.workspaceResolver.getWorkspaceFolder(undefined, environment);
    if (!folder) {
      return known;
    }

    const snapshot = await this.loadWorkspace(folder.fsPath, forceRefresh);
    return this.findRecord(snapshot, environment);
  }

  /** Return already-discovered Riot data for synchronous API hooks. */
  getKnownEnvironmentRecord(
    environment: PythonEnvironment,
  ): RiotEnvironmentRecord | undefined {
    return this.findKnownRecord(environment);
  }

  /** Get venv indexes by workspace for the indicator service. */
  async getVenvIndexesByWorkspace(): Promise<Map<string, Map<string, RtVenv>>> {
    const folders = this.workspaceResolver.getWorkspaceFolders();
    const snapshots = await Promise.all(
      folders.map((folder) => this.loadWorkspace(folder.fsPath, false)),
    );
    const indexes = new Map<string, Map<string, RtVenv>>();
    folders.forEach((folder, index) => {
      const venvs = snapshots[index].venvs;
      if (venvs.length > 0) {
        indexes.set(folder.fsPath, this.buildVenvIndex(venvs));
      }
    });
    return indexes;
  }

  /** Build a stable Python environment object from Riot JSON output. */
  buildEnvironment(
    venv: RtVenv,
    context: RtExecutionContext,
    workspaceRoot?: string,
  ): PythonEnvironment {
    const displayPath =
      workspaceRoot && path.isAbsolute(context.venv_path)
        ? path.relative(workspaceRoot, context.venv_path)
        : context.venv_path;
    const activation = this.buildShellActivation(context.activate_path);

    return {
      name: venv.name,
      displayName: context.display_name,
      shortDisplayName: context.short_display_name,
      displayPath,
      version: venv.python,
      group: venv.name,
      environmentPath: vscode.Uri.file(context.venv_path),
      tooltip: `Riot context ${context.hash}${workspaceRoot ? ` in ${workspaceRoot}` : ""}`,
      execInfo: {
        run: { executable: context.python_path },
        activatedRun: { executable: context.python_path },
        activation: activation.get("unknown"),
        deactivation: [{ executable: "deactivate" }],
        shellActivation: activation,
      },
      sysPrefix: context.venv_path,
      envId: {
        id: vscode.Uri.file(path.resolve(context.venv_path)).toString(),
        managerId: this.managerId,
      },
    };
  }

  private async loadWorkspace(
    cwd: string,
    forceRefresh: boolean,
  ): Promise<WorkspaceSnapshot> {
    if (!forceRefresh) {
      const cached = this.snapshots.get(cwd);
      if (cached) {
        return cached;
      }
    }

    const ongoing = this.ongoingLoads.get(cwd);
    if (ongoing) {
      return ongoing;
    }

    const previous = this.snapshots.get(cwd);
    let load!: Promise<WorkspaceSnapshot>;
    load = this.cliService
      .listEnvironments(cwd)
      .then((venvs) => {
        const snapshot = this.buildSnapshot(cwd, venvs);
        this.snapshots.set(cwd, snapshot);
        return snapshot;
      })
      .catch((error: unknown) => {
        const message = error instanceof Error ? error.message : String(error);
        this.log.error(`Failed to fetch Riot environments for ${cwd}: ${message}`);
        if (previous) {
          return previous;
        }
        const empty = this.buildSnapshot(cwd, []);
        this.snapshots.set(cwd, empty);
        return empty;
      })
      .finally(() => {
        if (this.ongoingLoads.get(cwd) === load) {
          this.ongoingLoads.delete(cwd);
        }
      });
    this.ongoingLoads.set(cwd, load);
    return load;
  }

  private buildSnapshot(cwd: string, venvs: RtVenv[]): WorkspaceSnapshot {
    const environments: PythonEnvironment[] = [];
    const recordsById = new Map<string, RiotEnvironmentRecord>();
    const recordsByHash = new Map<string, RiotEnvironmentRecord>();

    for (const venv of venvs) {
      for (const context of venv.execution_contexts ?? []) {
        const environment = this.buildEnvironment(venv, context, cwd);
        const record: RiotEnvironmentRecord = {
          workspaceRoot: cwd,
          venv,
          context,
          environment,
        };
        environments.push(environment);
        recordsById.set(environment.envId.id, record);
        recordsByHash.set(context.hash, record);
      }
    }

    return { venvs, environments, recordsById, recordsByHash };
  }

  private async reconcileCurrentEnvironment(
    folder: vscode.Uri,
    snapshot: WorkspaceSnapshot,
  ): Promise<void> {
    const cwd = folder.fsPath;
    const current = this.currentEnvironment.get(cwd);
    if (!current) {
      return;
    }

    const updated = this.findRecord(snapshot, current)?.environment;
    if (updated && this.environmentExecutableExists(updated)) {
      this.currentEnvironment.set(cwd, updated);
      return;
    }

    this.currentEnvironment.delete(cwd);
    await this.testingConfigManager.clearConfiguration(folder);
    this.envEmitter.fire({ uri: folder, old: current, new: undefined });
  }

  private async clearEnvironment(
    cwd: string,
    folder: vscode.Uri,
    previous?: PythonEnvironment,
  ): Promise<void> {
    await this.testingConfigManager.clearConfiguration(folder);
    this.currentEnvironment.delete(cwd);
    if (previous) {
      this.envEmitter.fire({ uri: folder, old: previous, new: undefined });
    }
  }

  private async activateEnvironment(
    cwd: string,
    folder: vscode.Uri,
    requested: PythonEnvironment,
    previous?: PythonEnvironment,
  ): Promise<void> {
    let record = await this.getEnvironmentRecord(requested, false);
    if (!record || record.workspaceRoot !== cwd) {
      const snapshot = await this.loadWorkspace(cwd, false);
      record = this.findRecord(snapshot, requested);
    }
    if (!record) {
      throw new Error(`Riot environment not found: ${requested.displayName}`);
    }

    if (!(await this.environmentIsReady(record))) {
      const controller = new AbortController();
      this.ongoingActivations.set(cwd, controller);
      try {
        await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: `Building ${requested.displayName}`,
            cancellable: true,
          },
          async (progress, token) => {
            token.onCancellationRequested(() => controller.abort());
            progress.report({ message: "Running rt build" });
            await this.cliService.buildEnvironment(record!.context.hash, cwd, {
              signal: controller.signal,
            });
          },
        );
      } finally {
        if (this.ongoingActivations.get(cwd) === controller) {
          this.ongoingActivations.delete(cwd);
        }
      }

      const refreshed = await this.loadWorkspace(cwd, true);
      record = this.findRecord(refreshed, requested);
      if (!record || !(await this.environmentIsReady(record))) {
        throw new Error(
          `Environment not ready after build: ${requested.displayName}`,
        );
      }
    }

    await this.testingConfigManager.updateConfiguration(folder, record.context);
    this.currentEnvironment.set(cwd, record.environment);
    if (previous?.envId.id !== record.environment.envId.id) {
      this.envEmitter.fire({ uri: folder, old: previous, new: record.environment });
    }
  }

  private cancelActivation(cwd: string): void {
    const controller = this.ongoingActivations.get(cwd);
    if (controller) {
      this.log.info(`Cancelling previous environment activation for ${cwd}`);
      controller.abort();
    }
  }

  private findKnownRecord(
    environment: PythonEnvironment,
  ): RiotEnvironmentRecord | undefined {
    for (const snapshot of this.snapshots.values()) {
      const record = this.findRecord(snapshot, environment);
      if (record) {
        return record;
      }
    }
    return undefined;
  }

  private findRecord(
    snapshot: WorkspaceSnapshot,
    environment: PythonEnvironment,
  ): RiotEnvironmentRecord | undefined {
    return (
      snapshot.recordsById.get(environment.envId.id) ??
      snapshot.recordsByHash.get(environment.envId.id) ??
      Array.from(snapshot.recordsById.values()).find((record) =>
        this.matchesPath(record.environment, environment.environmentPath.fsPath),
      )
    );
  }

  private computeChanges(
    previous: PythonEnvironment[],
    next: PythonEnvironment[],
  ): DidChangeEnvironmentsEventArgs {
    const changes: DidChangeEnvironmentsEventArgs = [];
    const previousIds = new Set(previous.map((environment) => environment.envId.id));
    const nextIds = new Set(next.map((environment) => environment.envId.id));

    for (const environment of next) {
      if (!previousIds.has(environment.envId.id)) {
        changes.push({ kind: EnvironmentChangeKind.add, environment });
      }
    }
    for (const environment of previous) {
      if (!nextIds.has(environment.envId.id)) {
        changes.push({ kind: EnvironmentChangeKind.remove, environment });
      }
    }
    return changes;
  }

  private buildVenvIndex(venvs: RtVenv[]): Map<string, RtVenv> {
    const index = new Map<string, RtVenv>();
    for (const venv of venvs) {
      index.set(venv.hash, venv);
      for (const context of venv.execution_contexts ?? []) {
        index.set(context.hash, venv);
      }
    }
    return index;
  }

  private buildShellActivation(
    activatePath: string,
  ): Map<string, PythonCommandRunConfiguration[]> {
    const binDirectory = path.dirname(activatePath);
    const posix = [{ executable: "source", args: [activatePath] }];
    return new Map<string, PythonCommandRunConfiguration[]>([
      ["unknown", posix],
      ["sh", posix],
      ["bash", posix],
      ["gitbash", posix],
      ["zsh", posix],
      ["ksh", [{ executable: ".", args: [activatePath] }]],
      [
        "fish",
        [
          {
            executable: "source",
            args: [path.join(binDirectory, "activate.fish")],
          },
        ],
      ],
      [
        "csh",
        [
          {
            executable: "source",
            args: [path.join(binDirectory, "activate.csh")],
          },
        ],
      ],
      [
        "tcsh",
        [
          {
            executable: "source",
            args: [path.join(binDirectory, "activate.csh")],
          },
        ],
      ],
    ]);
  }

  private environmentExecutableExists(environment: PythonEnvironment): boolean {
    return fs.existsSync(environment.execInfo.run.executable);
  }

  private async environmentIsReady(record: RiotEnvironmentRecord): Promise<boolean> {
    return (
      this.environmentExecutableExists(record.environment) &&
      (await this.cliService.isVenvReady(record.context.hash, record.workspaceRoot))
    );
  }

  private matchesPath(environment: PythonEnvironment, targetPath: string): boolean {
    const target = this.normalizeFsPath(targetPath);
    const environmentPath = this.normalizeFsPath(
      environment.environmentPath.fsPath,
    );
    const executable = this.normalizeFsPath(environment.execInfo.run.executable);
    return (
      target === environmentPath ||
      target === executable ||
      target === this.normalizeFsPath(path.dirname(executable))
    );
  }

  private normalizeFsPath(value: string): string {
    const normalized = path.resolve(value);
    return process.platform === "win32" ? normalized.toLowerCase() : normalized;
  }
}
