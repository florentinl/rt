import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  DidChangeEnvironmentEventArgs,
  DidChangeEnvironmentsEventArgs,
  EnvironmentChangeKind,
  EnvironmentGroupInfo,
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
import {
  buildManagerId,
  RtSuite,
  RtTestEnvironment,
  RtTestRun,
} from "./types/rtTypes";

export interface SuitespecEnvironmentRecord {
  readonly workspaceRoot: string;
  readonly suite: RtSuite;
  readonly testEnvironment: RtTestEnvironment;
  readonly run: RtTestRun;
  readonly environment: PythonEnvironment;
}

interface WorkspaceSnapshot {
  readonly suites: RtSuite[];
  readonly environments: PythonEnvironment[];
  readonly recordsById: Map<string, SuitespecEnvironmentRecord>;
  readonly recordsByRunId: Map<string, SuitespecEnvironmentRecord>;
}

/** Exposes suitespec test runs through VS Code's native Python Environments API. */
export class SuitespecEnvManager implements EnvironmentManager {
  readonly name = "rt";
  readonly displayName = "rt suites";
  readonly description = "Immutable Python test environments declared by suitespec";
  readonly tooltip = "Suitespec test environments managed by rt";
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
    this.preferredPackageManagerId = buildManagerId(extensionId, "rt-packages");
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
        const next = await this.loadWorkspace(cwd, true);
        changes.push(...this.computeChanges(previous, next.environments));
        await this.reconcileCurrentEnvironment(folder, next);
      }),
    );
    if (changes.length > 0) {
      this.envsEmitter.fire(changes);
    }
  }

  async getEnvironments(scope: GetEnvironmentsScope): Promise<PythonEnvironment[]> {
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
          "Unable to determine the workspace folder for this suite environment.",
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

  async get(scope: GetEnvironmentScope): Promise<PythonEnvironment | undefined> {
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
    const scoped = this.workspaceResolver.getWorkspaceFolder(context);
    const all = this.workspaceResolver.getWorkspaceFolders();
    const folders = scoped
      ? [scoped, ...all.filter((folder) => folder.toString() !== scoped.toString())]
      : all;
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

  async forceReinstallEnvironment(
    scope: GetEnvironmentScope,
    environment: PythonEnvironment,
  ): Promise<void> {
    const folder = this.workspaceResolver.getWorkspaceFolder(scope, environment);
    if (!folder) {
      throw new Error("Unable to determine the suitespec workspace");
    }
    const record = await this.getEnvironmentRecord(environment, false);
    if (!record) {
      throw new Error(`Suite environment not found: ${environment.displayName}`);
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
        await this.cliService.buildEnvironment(record.run.id, folder.fsPath, {
          forceReinstall: true,
          signal: controller.signal,
        });
      },
    );
    const refreshed = await this.loadWorkspace(folder.fsPath, true);
    const updated = this.findRecord(refreshed, environment)?.environment;
    if (updated && this.currentEnvironment.has(folder.fsPath)) {
      this.currentEnvironment.set(folder.fsPath, updated);
    }
  }

  async getEnvironmentRecord(
    environment: PythonEnvironment,
    forceRefresh: boolean,
  ): Promise<SuitespecEnvironmentRecord | undefined> {
    const known = this.findKnownRecord(environment);
    const folder = known
      ? vscode.Uri.file(known.workspaceRoot)
      : this.workspaceResolver.getWorkspaceFolder(undefined, environment);
    if (!folder) {
      return known;
    }
    return this.findRecord(
      await this.loadWorkspace(folder.fsPath, forceRefresh),
      environment,
    );
  }

  getKnownEnvironmentRecord(
    environment: PythonEnvironment,
  ): SuitespecEnvironmentRecord | undefined {
    return this.findKnownRecord(environment);
  }

  async getSuitesByWorkspace(): Promise<Map<string, RtSuite[]>> {
    const folders = this.workspaceResolver.getWorkspaceFolders();
    const snapshots = await Promise.all(
      folders.map((folder) => this.loadWorkspace(folder.fsPath, false)),
    );
    const result = new Map<string, RtSuite[]>();
    folders.forEach((folder, index) => {
      if (snapshots[index].suites.length > 0) {
        result.set(folder.fsPath, snapshots[index].suites);
      }
    });
    return result;
  }

  buildEnvironment(
    suite: RtSuite,
    testEnvironment: RtTestEnvironment,
    run: RtTestRun,
    workspaceRoot?: string,
  ): PythonEnvironment {
    const displayPath =
      workspaceRoot && path.isAbsolute(run.venv_path)
        ? path.relative(workspaceRoot, run.venv_path)
        : run.venv_path;
    const activation = this.buildShellActivation(run.activate_path);
    return {
      name: testEnvironment.name,
      displayName: run.display_name,
      shortDisplayName: run.short_display_name,
      displayPath,
      version: testEnvironment.python,
      group: this.buildSuiteGroup(suite),
      environmentPath: vscode.Uri.file(run.venv_path),
      description: testEnvironment.integration_name,
      tooltip: this.buildEnvironmentTooltip(suite, testEnvironment, run),
      execInfo: {
        run: { executable: run.python_path },
        activatedRun: { executable: run.python_path },
        activation: activation.get("unknown"),
        deactivation: [{ executable: "deactivate" }],
        shellActivation: activation,
      },
      sysPrefix: testEnvironment.venv_path,
      envId: {
        id: vscode.Uri.file(path.resolve(run.venv_path)).toString(),
        managerId: this.managerId,
      },
    };
  }

  private buildSuiteGroup(suite: RtSuite): EnvironmentGroupInfo {
    const variants = suite.environments.length;
    const services = suite.services.length
      ? ` · services: ${suite.services.join(", ")}`
      : "";
    const tooltip = [
      `Suite: ${suite.id}`,
      suite.source_files.length
        ? `Sources: ${suite.source_files.join(", ")}`
        : undefined,
      suite.patterns.length ? `Patterns: ${suite.patterns.join(", ")}` : undefined,
      suite.services.length ? `Services: ${suite.services.join(", ")}` : undefined,
      Object.keys(suite.suite_env).length
        ? `Environment: ${formatRecord(suite.suite_env)}`
        : undefined,
    ]
      .filter((line): line is string => line !== undefined)
      .join("\n");
    return {
      name: suite.id,
      description: `${variants} dependency environment${variants === 1 ? "" : "s"}${services}`,
      tooltip,
    };
  }

  private buildEnvironmentTooltip(
    suite: RtSuite,
    environment: RtTestEnvironment,
    run: RtTestRun,
  ): string {
    return [
      `Suite: ${suite.id}`,
      `Dependency environment: ${environment.name} (${environment.hash})`,
      `Python: ${environment.python}`,
      `Run: ${run.index + 1} (${run.id})`,
      environment.direct_dependencies.length
        ? `Direct dependencies: ${environment.direct_dependencies.join(", ")}`
        : "Direct dependencies: none",
      `Lockfile: ${environment.lockfile}`,
      Object.keys(run.env).length ? `Run environment: ${formatRecord(run.env)}` : undefined,
      `Profile: ${run.profile_path}`,
    ]
      .filter((line): line is string => line !== undefined)
      .join("\n");
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
      .listSuites(cwd)
      .then((suites) => {
        const snapshot = this.buildSnapshot(cwd, suites);
        this.snapshots.set(cwd, snapshot);
        return snapshot;
      })
      .catch((error: unknown) => {
        const message = error instanceof Error ? error.message : String(error);
        this.log.error(`Failed to fetch suitespec environments for ${cwd}: ${message}`);
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

  private buildSnapshot(cwd: string, suites: RtSuite[]): WorkspaceSnapshot {
    const environments: PythonEnvironment[] = [];
    const recordsById = new Map<string, SuitespecEnvironmentRecord>();
    const recordsByRunId = new Map<string, SuitespecEnvironmentRecord>();
    for (const suite of suites) {
      for (const testEnvironment of suite.environments) {
        for (const run of testEnvironment.runs) {
          const environment = this.buildEnvironment(
            suite,
            testEnvironment,
            run,
            cwd,
          );
          const record = { workspaceRoot: cwd, suite, testEnvironment, run, environment };
          environments.push(environment);
          recordsById.set(environment.envId.id, record);
          recordsByRunId.set(run.id, record);
        }
      }
    }
    return { suites, environments, recordsById, recordsByRunId };
  }

  private async reconcileCurrentEnvironment(
    folder: vscode.Uri,
    snapshot: WorkspaceSnapshot,
  ): Promise<void> {
    const current = this.currentEnvironment.get(folder.fsPath);
    if (!current) {
      return;
    }
    const updated = this.findRecord(snapshot, current)?.environment;
    if (updated && this.environmentExecutableExists(updated)) {
      this.currentEnvironment.set(folder.fsPath, updated);
      return;
    }
    this.currentEnvironment.delete(folder.fsPath);
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
      record = this.findRecord(await this.loadWorkspace(cwd, false), requested);
    }
    if (!record) {
      throw new Error(`Suite environment not found: ${requested.displayName}`);
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
            await this.cliService.buildEnvironment(record!.run.id, cwd, {
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
        throw new Error(`Environment not ready after build: ${requested.displayName}`);
      }
    }
    await this.testingConfigManager.updateConfiguration(folder, record.run);
    this.currentEnvironment.set(cwd, record.environment);
    if (previous?.envId.id !== record.environment.envId.id) {
      this.envEmitter.fire({ uri: folder, old: previous, new: record.environment });
    }
  }

  private cancelActivation(cwd: string): void {
    const controller = this.ongoingActivations.get(cwd);
    if (controller) {
      this.log.info(`Cancelling previous suite environment activation for ${cwd}`);
      controller.abort();
    }
  }

  private findKnownRecord(
    environment: PythonEnvironment,
  ): SuitespecEnvironmentRecord | undefined {
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
  ): SuitespecEnvironmentRecord | undefined {
    return (
      snapshot.recordsById.get(environment.envId.id) ??
      snapshot.recordsByRunId.get(environment.envId.id) ??
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
      ["fish", [{ executable: "source", args: [path.join(binDirectory, "activate.fish")] }]],
      ["csh", [{ executable: "source", args: [path.join(binDirectory, "activate.csh")] }]],
      ["tcsh", [{ executable: "source", args: [path.join(binDirectory, "activate.csh")] }]],
    ]);
  }

  private environmentExecutableExists(environment: PythonEnvironment): boolean {
    return fs.existsSync(environment.execInfo.run.executable);
  }

  private async environmentIsReady(
    record: SuitespecEnvironmentRecord,
  ): Promise<boolean> {
    return (
      this.environmentExecutableExists(record.environment) &&
      (await this.cliService.isEnvironmentReady(
        record.run.id,
        record.run.profile_fingerprint,
        record.workspaceRoot,
      ))
    );
  }

  private matchesPath(environment: PythonEnvironment, targetPath: string): boolean {
    const target = normalizeFsPath(targetPath);
    const environmentPath = normalizeFsPath(environment.environmentPath.fsPath);
    const executable = normalizeFsPath(environment.execInfo.run.executable);
    return (
      target === environmentPath ||
      target === executable ||
      target === normalizeFsPath(path.dirname(executable))
    );
  }
}

function normalizeFsPath(value: string): string {
  const normalized = path.resolve(value);
  return process.platform === "win32" ? normalized.toLowerCase() : normalized;
}

function formatRecord(values: Record<string, string>): string {
  return Object.entries(values)
    .map(([name, value]) => `${name}=${value}`)
    .join(", ");
}
