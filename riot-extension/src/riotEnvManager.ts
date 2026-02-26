/**
 * Riot Environment Manager
 * Coordinates environment discovery, management, and lifecycle
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
  PythonEnvironment,
  RefreshEnvironmentsScope,
  ResolveEnvironmentContext,
  SetEnvironmentScope,
} from "./api";
import { EnvironmentDisplayFormatter } from "./formatters/environmentFormatter";
import { RtCliService } from "./services/rtCliService";
import { StatePersistenceService } from "./services/statePersistence";
import { TestingConfigurationManager } from "./services/testingConfigManager";
import { WorkspaceResolver } from "./services/workspaceResolver";
import { buildManagerId, RtExecutionContext, RtVenv } from "./types/rtTypes";

export { RtExecutionContext, RtVenv } from "./types/rtTypes";

/**
 * Main environment manager implementation
 */
export class RiotEnvManager implements EnvironmentManager {
  name = "riot";
  displayName = "Riot";
  description?: string;
  tooltip?: string | vscode.MarkdownString;
  iconPath?: IconPath;
  log?: vscode.LogOutputChannel;
  readonly managerId: string;
  readonly preferredPackageManagerId: string;

  private readonly cliService: RtCliService;
  private readonly workspaceResolver: WorkspaceResolver;
  private readonly testingConfigManager: TestingConfigurationManager;
  private readonly statePersistence: StatePersistenceService;
  private readonly formatter: EnvironmentDisplayFormatter;
  private readonly currentEnvironment = new Map<string, PythonEnvironment>();
  private readonly knownEnvironments = new Map<string, PythonEnvironment[]>();
  private readonly ongoingActivations = new Map<string, AbortController>();

  private readonly envsEmitter =
    new vscode.EventEmitter<DidChangeEnvironmentsEventArgs>();
  private readonly envEmitter =
    new vscode.EventEmitter<DidChangeEnvironmentEventArgs>();

  onDidChangeEnvironments = this.envsEmitter.event;
  onDidChangeEnvironment = this.envEmitter.event;

  constructor(
    log: vscode.LogOutputChannel,
    extensionId: string,
    workspaceState: vscode.Memento,
    rtPath?: string,
  ) {
    this.managerId = buildManagerId(extensionId, this.name);
    this.preferredPackageManagerId = buildManagerId(extensionId, "rt");
    this.log = log;

    this.cliService = new RtCliService(rtPath, log);
    this.workspaceResolver = new WorkspaceResolver();
    this.testingConfigManager = new TestingConfigurationManager(log);
    this.statePersistence = new StatePersistenceService(workspaceState);
    this.formatter = new EnvironmentDisplayFormatter();
  }

  async refresh(scope: RefreshEnvironmentsScope): Promise<void> {
    const folders = this.workspaceResolver.getWorkspaceFoldersForScope(scope);
    if (folders.length === 0) {
      return;
    }

    const changes: DidChangeEnvironmentsEventArgs = [];
    for (const folder of folders) {
      const cwd = folder.fsPath;
      const previous = this.knownEnvironments.get(cwd) ?? [];
      const next = await this.fetchEnvironments(cwd);
      this.knownEnvironments.set(cwd, next);
      changes.push(...this.computeChanges(previous, next));

      const current = this.currentEnvironment.get(cwd);
      if (!current) {
        continue;
      }

      const updated = next.find((env) => env.envId.id === current.envId.id);
      if (updated && this.environmentExecutableExists(updated)) {
        this.currentEnvironment.set(cwd, updated);
        continue;
      }

      await this.statePersistence.saveLastEnvironment(cwd, undefined);
      this.currentEnvironment.delete(cwd);
      this.envEmitter.fire({ uri: folder, old: current, new: undefined });
    }

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
    return this.fetchEnvironmentsForFolders(folders);
  }

  async set(
    scope: SetEnvironmentScope,
    environment?: PythonEnvironment,
  ): Promise<void> {
    const folder = this.workspaceResolver.getWorkspaceFolder(
      scope,
      environment,
    );
    if (!folder) {
      if (environment) {
        void vscode.window.showErrorMessage(
          "Unable to determine workspace folder. Open a file in the target workspace and try again.",
        );
      }
      return;
    }

    const cwd = folder.fsPath;
    const previous = this.currentEnvironment.get(cwd);

    // Cancel any ongoing activation for this workspace
    const ongoingController = this.ongoingActivations.get(cwd);
    if (ongoingController) {
      this.log?.info(`Cancelling previous environment activation for ${cwd}`);
      ongoingController.abort();
    }

    if (!environment) {
      await this.clearEnvironment(cwd, folder, previous);
      return;
    }

    await this.activateEnvironment(cwd, folder, environment, previous);
  }

  async get(
    scope: GetEnvironmentScope,
  ): Promise<PythonEnvironment | undefined> {
    const folder = this.workspaceResolver.getWorkspaceFolder(scope);
    if (!folder) {
      return undefined;
    }

    const cwd = folder.fsPath;
    const current = this.currentEnvironment.get(cwd);
    if (current && this.environmentExecutableExists(current)) {
      return current;
    }

    this.currentEnvironment.delete(cwd);
    return this.restoreEnvironment(cwd);
  }

  async resolve(
    context: ResolveEnvironmentContext,
  ): Promise<PythonEnvironment | undefined> {
    const targetPath = context.fsPath;
    const folder = this.workspaceResolver.getWorkspaceFolder(context);

    if (folder) {
      const envs = await this.fetchEnvironments(folder.fsPath);
      const match = envs.find((env) => this.matchesPath(env, targetPath));
      if (match) {
        return match;
      }
    }

    // Search all workspaces
    const folders = this.workspaceResolver.getWorkspaceFolders();
    for (const f of folders) {
      const envs = await this.fetchEnvironments(f.fsPath);
      const match = envs.find((env) => this.matchesPath(env, targetPath));
      if (match) {
        return match;
      }
    }

    return undefined;
  }

  async clearCache?(): Promise<void> {
    this.currentEnvironment.clear();
    this.knownEnvironments.clear();
  }

  /**
   * Force reinstall an environment
   */
  async forceReinstallEnvironment(
    scope: GetEnvironmentScope,
    environment: PythonEnvironment,
  ): Promise<void> {
    const folder = this.workspaceResolver.getWorkspaceFolder(
      scope,
      environment,
    );
    const cwd = folder?.fsPath ?? environment.environmentPath.fsPath;

    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: `Rebuilding ${environment.displayName}`,
        cancellable: true,
      },
      async (progress, token) => {
        const controller = new AbortController();
        token.onCancellationRequested(() => controller.abort());

        try {
          progress.report({ message: "Running rt build --force-reinstall" });
          await this.cliService.buildEnvironment(environment.envId.id, cwd, {
            forceReinstall: true,
            signal: controller.signal,
          });
        } catch (err) {
          if (err instanceof vscode.CancellationError) {
            this.log?.info("Rebuild cancelled");
            return;
          }
          const message = err instanceof Error ? err.message : String(err);
          this.log?.error(`Rebuild failed: ${message}`);
          void vscode.window.showErrorMessage(`Rebuild failed: ${message}`);
          throw err;
        }
      },
    );
  }

  /**
   * Get venv indexes by workspace (for indicator service)
   */
  async getVenvIndexesByWorkspace(): Promise<Map<string, Map<string, RtVenv>>> {
    const folders = this.workspaceResolver.getWorkspaceFolders();
    const indexes = new Map<string, Map<string, RtVenv>>();

    for (const folder of folders) {
      const venvs = await this.cliService.listEnvironments(folder.fsPath);
      indexes.set(folder.fsPath, this.buildVenvIndex(venvs));
    }

    return indexes;
  }

  /**
   * Get venv by hash (for package manager)
   */
  async getVenvByHash(hash: string): Promise<RtVenv | undefined> {
    const folders = this.workspaceResolver.getWorkspaceFolders();
    for (const folder of folders) {
      const venvs = await this.cliService.listEnvironments(folder.fsPath);
      const venv = this.findVenvByHash(venvs, hash);
      if (venv) {
        return venv;
      }
    }

    return undefined;
  }

  /**
   * Build a PythonEnvironment from venv and context
   */
  buildEnvironment(
    venv: RtVenv,
    ctx: RtExecutionContext,
    workspaceRoot?: string,
  ): PythonEnvironment {
    const pythonPath = this.getPythonPath(ctx.venv_path);
    const { activate, deactivate } = this.getActivationPaths(ctx.venv_path);
    const { displayName, shortDisplayName } = this.formatter.buildDisplayNames(
      venv,
      ctx,
    );
    const displayPath =
      workspaceRoot && path.isAbsolute(ctx.venv_path)
        ? path.relative(workspaceRoot, ctx.venv_path)
        : ctx.venv_path;

    return {
      name: venv.name,
      displayName,
      shortDisplayName,
      displayPath,
      version: venv.python,
      environmentPath: vscode.Uri.file(ctx.venv_path),
      tooltip: `Environment hash: ${ctx.hash}`,
      execInfo: {
        run: { executable: pythonPath },
        activatedRun: { executable: pythonPath },
        activation:
          process.platform === "win32"
            ? [{ executable: activate }]
            : [{ executable: "source", args: [activate] }],
        deactivation:
          process.platform === "win32"
            ? [{ executable: deactivate }]
            : [{ executable: "deactivate", args: [] }],
      },
      sysPrefix: ctx.venv_path,
      envId: {
        id: ctx.hash,
        managerId: this.managerId,
      },
    };
  }

  // ============ Private Methods ============

  private async fetchEnvironmentsForFolders(
    folders: vscode.Uri[],
  ): Promise<PythonEnvironment[]> {
    const results = await Promise.all(
      folders.map((f) => this.fetchEnvironments(f.fsPath)),
    );
    return results.flat();
  }

  private async fetchEnvironments(cwd: string): Promise<PythonEnvironment[]> {
    try {
      const venvs = await this.cliService.listEnvironments(cwd);
      const environments: PythonEnvironment[] = [];

      for (const venv of venvs) {
        venv.python = this.formatter.normalizePythonVersion(venv.python);
        for (const ctx of venv.execution_contexts ?? []) {
          environments.push(this.buildEnvironment(venv, ctx, cwd));
        }
      }

      return environments;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.log?.error(`Failed to fetch environments: ${message}`);
      return [];
    }
  }

  private async clearEnvironment(
    cwd: string,
    folder: vscode.Uri,
    previous?: PythonEnvironment,
  ): Promise<void> {
    await this.testingConfigManager.clearConfiguration(folder);
    await this.statePersistence.saveLastEnvironment(cwd, undefined);
    this.currentEnvironment.delete(cwd);
    this.envEmitter.fire({ uri: folder, old: previous, new: undefined });
  }

  private async activateEnvironment(
    cwd: string,
    folder: vscode.Uri,
    environment: PythonEnvironment,
    previous?: PythonEnvironment,
  ): Promise<void> {
    const controller = new AbortController();
    this.ongoingActivations.set(cwd, controller);

    try {
      await vscode.window.withProgress(
        {
          location: vscode.ProgressLocation.Notification,
          title: `Switching to ${environment.displayName}`,
        },
        async (progress) => {
          progress.report({ message: "Building environment" });
          await this.cliService.buildEnvironment(environment.envId.id, cwd, {
            signal: controller.signal,
          });

          progress.report({ message: "Loading built environment" });
          const venvs = await this.cliService.listEnvironments(cwd);
          const venv = this.findVenvByHash(venvs, environment.envId.id);
          if (!venv) {
            throw new Error(
              `Environment ${environment.envId.id} not found after build`,
            );
          }
          const ctx = venv?.execution_contexts.find(
            (c) => c.hash === environment.envId.id,
          );
          if (!ctx) {
            throw new Error(
              `Execution context ${environment.envId.id} not found after build`,
            );
          }
          const activated = this.buildEnvironment(venv, ctx, cwd);
          if (!this.environmentExecutableExists(activated)) {
            throw new Error(
              `Environment interpreter not found: ${activated.execInfo.run.executable}`,
            );
          }

          progress.report({ message: "Updating test configuration" });
          await this.testingConfigManager.updateConfiguration(folder, ctx);

          this.currentEnvironment.set(cwd, activated);
          await this.statePersistence.saveLastEnvironment(
            cwd,
            activated.envId.id,
          );
          this.envEmitter.fire({
            uri: folder,
            old: previous,
            new: activated,
          });
        },
      );
    } catch (err) {
      if (err instanceof vscode.CancellationError) {
        this.log?.info(
          `Environment activation cancelled for ${environment.displayName}`,
        );
        return;
      }
      throw err;
    } finally {
      // Clean up controller if it's still ours
      if (this.ongoingActivations.get(cwd) === controller) {
        this.ongoingActivations.delete(cwd);
      }
    }
  }

  private async restoreEnvironment(
    cwd: string,
  ): Promise<PythonEnvironment | undefined> {
    const savedEnvId = this.statePersistence.getLastEnvironment(cwd);
    if (!savedEnvId) {
      return undefined;
    }

    try {
      await this.cliService.buildEnvironment(savedEnvId, cwd);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.log?.warn(`Failed to restore environment ${savedEnvId}: ${message}`);
      await this.statePersistence.saveLastEnvironment(cwd, undefined);
      this.currentEnvironment.delete(cwd);
      return undefined;
    }

    const restored = await this.findEnvironmentById(cwd, savedEnvId);

    if (!restored || !this.environmentExecutableExists(restored)) {
      this.log?.warn(`Restored environment ${savedEnvId} is not usable`);
      await this.statePersistence.saveLastEnvironment(cwd, undefined);
      this.currentEnvironment.delete(cwd);
      return undefined;
    }

    this.currentEnvironment.set(cwd, restored);
    return restored;
  }

  private computeChanges(
    previous: PythonEnvironment[],
    next: PythonEnvironment[],
  ): DidChangeEnvironmentsEventArgs {
    const changes: DidChangeEnvironmentsEventArgs = [];
    const prevIds = new Set(previous.map((e) => e.envId.id));
    const nextIds = new Set(next.map((e) => e.envId.id));

    for (const env of next) {
      if (!prevIds.has(env.envId.id)) {
        changes.push({ kind: EnvironmentChangeKind.add, environment: env });
      }
    }

    for (const env of previous) {
      if (!nextIds.has(env.envId.id)) {
        changes.push({ kind: EnvironmentChangeKind.remove, environment: env });
      }
    }

    return changes;
  }

  private buildVenvIndex(venvs: RtVenv[]): Map<string, RtVenv> {
    const index = new Map<string, RtVenv>();
    for (const venv of venvs) {
      index.set(venv.hash, venv);
      for (const ctx of venv.execution_contexts ?? []) {
        index.set(ctx.hash, venv);
      }
    }
    return index;
  }

  private findVenvByHash(venvs: RtVenv[], hash: string): RtVenv | undefined {
    for (const venv of venvs) {
      if (venv.hash === hash) {
        return venv;
      }
      if (venv.execution_contexts?.some((ctx) => ctx.hash === hash)) {
        return venv;
      }
    }
    return undefined;
  }

  private async findEnvironmentById(
    cwd: string,
    envId: string,
  ): Promise<PythonEnvironment | undefined> {
    const envs = await this.fetchEnvironments(cwd);
    return envs.find((env) => env.envId.id === envId);
  }

  private environmentExecutableExists(environment: PythonEnvironment): boolean {
    return fs.existsSync(environment.execInfo.run.executable);
  }

  private matchesPath(env: PythonEnvironment, targetPath: string): boolean {
    return (
      env.environmentPath.fsPath === targetPath ||
      env.execInfo.run.executable === targetPath ||
      path.dirname(env.execInfo.run.executable) === targetPath
    );
  }

  private getPythonPath(venvPath: string): string {
    const binDir = process.platform === "win32" ? "Scripts" : "bin";
    const pythonBin = process.platform === "win32" ? "python.exe" : "python";
    return path.join(venvPath, binDir, pythonBin);
  }

  private getActivationPaths(venvPath: string) {
    const binDir = process.platform === "win32" ? "Scripts" : "bin";
    const activateScript =
      process.platform === "win32" ? "activate.bat" : "activate";
    return {
      activate: path.join(venvPath, binDir, activateScript),
      deactivate: "deactivate",
    };
  }
}
