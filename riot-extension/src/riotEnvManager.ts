import * as cp from 'child_process';
import * as path from 'path';
import * as vscode from 'vscode';
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
} from './api';

const MAX_BUFFER_BYTES = 20 * 1024 * 1024; // 20MB to handle large rt list outputs

const normalizeManagerName = (name: string): string => name.toLowerCase().replace(/[^a-zA-Z0-9-_]/g, '_');
export const buildManagerId = (extensionId: string, name: string): string =>
    `${extensionId}:${normalizeManagerName(name)}`;

export type RtExecutionContext = {
    hash: string;
    venv_path: string;
    command?: string;
    pytest_target?: string;
    env: Record<string, string>;
    create: boolean;
    skip_dev_install: boolean;
};

export type RtVenv = {
    hash: string;
    venv_path: string;
    name: string;
    python: string;
    pkgs: Record<string, string>;
    shared_pkgs: Record<string, string>;
    shared_env: Record<string, string>;
    execution_contexts: RtExecutionContext[];
};

const isUri = (value: unknown): value is vscode.Uri => value instanceof vscode.Uri;

export class RiotEnvManager implements EnvironmentManager {
    name: string;
    displayName?: string | undefined;
    preferredPackageManagerId: string;
    description?: string | undefined;
    tooltip?: string | vscode.MarkdownString | undefined;
    iconPath?: IconPath | undefined;
    log?: vscode.LogOutputChannel | undefined;
    readonly managerId: string;
    private readonly packageManagerId: string;
    private readonly workspaceState: vscode.Memento;
    private readonly rtPath?: string;
    private readonly venvIndexByCwd = new Map<string, Map<string, RtVenv>>();
    private readonly cachedVenvsRawByCwd = new Map<string, RtVenv[]>();
    private readonly cachedEnvironmentsByCwd = new Map<string, PythonEnvironment[]>();
    private readonly currentEnvironmentByCwd = new Map<string, PythonEnvironment | undefined>();
    private readonly envWorkspaceRoots = new Map<string, string>();
    private readonly envsEmitter = new vscode.EventEmitter<DidChangeEnvironmentsEventArgs>();
    private readonly envEmitter = new vscode.EventEmitter<DidChangeEnvironmentEventArgs>();
    private readonly currentSetCancellationByCwd = new Map<string, AbortController>();
    private readonly currentSetPromiseByCwd = new Map<string, Promise<void>>();

    onDidChangeEnvironments?: vscode.Event<DidChangeEnvironmentsEventArgs> = this.envsEmitter.event;
    onDidChangeEnvironment?: vscode.Event<DidChangeEnvironmentEventArgs> = this.envEmitter.event;

    constructor(log: vscode.LogOutputChannel, extensionId: string, workspaceState: vscode.Memento, rtPath?: string) {
        this.name = 'riot';
        this.displayName = 'Riot';
        this.managerId = buildManagerId(extensionId, this.name);
        this.packageManagerId = buildManagerId(extensionId, 'rt');
        this.preferredPackageManagerId = this.packageManagerId;
        this.log = log;
        this.workspaceState = workspaceState;
        this.rtPath = rtPath;
    }

    async refresh(scope: RefreshEnvironmentsScope): Promise<void> {
        const folders = this.workspaceFoldersForScope(scope);
        if (folders.length === 0) {
            return;
        }
        const previous = this.cachedEnvironmentsForFolders(folders);
        const next = await this.fetchEnvironmentsForFolders(folders, true);

        const nextIds = new Set(next.map((env) => env.envId.id));
        const prevIds = new Set(previous.map((env) => env.envId.id));

        const changes: DidChangeEnvironmentsEventArgs = [];

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

        if (changes.length > 0) {
            this.envsEmitter.fire(changes);
        }
    }

    async getEnvironments(scope: GetEnvironmentsScope): Promise<PythonEnvironment[]> {
        if (scope === 'global') {
            return [];
        }
        const folders = this.workspaceFoldersForScope(scope);
        if (folders.length === 0) {
            return [];
        }
        return this.fetchEnvironmentsForFolders(folders);
    }

    set(scope: SetEnvironmentScope, environment?: PythonEnvironment): Promise<void> {
        return this.setEnvironment(scope, environment);
    }

    async get(scope: GetEnvironmentScope): Promise<PythonEnvironment | undefined> {
        const cwd = this.cwdForScope(scope);
        if (cwd) {
            const current = this.currentEnvironmentByCwd.get(cwd);
            if (current) {
                return current;
            }
        }
        return this.tryRestoreEnvironment(scope);
    }

    resolve(context: ResolveEnvironmentContext): Promise<PythonEnvironment | undefined> {
        return this.resolveEnvironment(context);
    }

    clearCache?(): Promise<void> {
        this.cachedEnvironmentsByCwd.clear();
        this.currentEnvironmentByCwd.clear();
        this.cachedVenvsRawByCwd.clear();
        this.venvIndexByCwd.clear();
        this.envWorkspaceRoots.clear();
        this.currentSetCancellationByCwd.clear();
        this.currentSetPromiseByCwd.clear();
        return Promise.resolve();
    }

    private workspaceFolders(): vscode.Uri[] {
        return vscode.workspace.workspaceFolders?.map((folder) => folder.uri) ?? [];
    }

    private workspaceFolderForUri(uri: vscode.Uri): vscode.Uri | undefined {
        return vscode.workspace.getWorkspaceFolder(uri)?.uri;
    }

    private workspaceFolderFromScopeOnly(scope: SetEnvironmentScope | GetEnvironmentScope): vscode.Uri | undefined {
        if (Array.isArray(scope) && scope.length > 0 && isUri(scope[0])) {
            const activeFolder = this.activeWorkspaceFolder();
            if (activeFolder) {
                const matchesActive = scope.some((uri) => {
                    const folder = this.workspaceFolderForUri(uri) ?? uri;
                    return folder.fsPath === activeFolder.fsPath;
                });
                if (matchesActive) {
                    return activeFolder;
                }
            }
            if (scope.length === 1) {
                return this.workspaceFolderForUri(scope[0]) ?? scope[0];
            }
            return undefined;
        }
        if (isUri(scope)) {
            return this.workspaceFolderForUri(scope) ?? scope;
        }
        return undefined;
    }

    private activeWorkspaceFolder(): vscode.Uri | undefined {
        const activeUri = vscode.window.activeTextEditor?.document.uri;
        return activeUri ? this.workspaceFolderForUri(activeUri) : undefined;
    }

    private workspaceFolderForEnvironment(environment: PythonEnvironment): vscode.Uri | undefined {
        const mappedRoot = this.envWorkspaceRoots.get(environment.envId.id);
        if (mappedRoot) {
            return vscode.Uri.file(mappedRoot);
        }
        const folder = vscode.workspace.getWorkspaceFolder(environment.environmentPath)?.uri;
        if (folder) {
            return folder;
        }
        const folders = this.workspaceFolders();
        if (folders.length === 1) {
            return folders[0];
        }
        return undefined;
    }

    private workspaceFolderForScope(
        scope: SetEnvironmentScope | GetEnvironmentScope,
        environment?: PythonEnvironment,
    ): vscode.Uri | undefined {
        const scopeFolder = this.workspaceFolderFromScopeOnly(scope);
        if (scopeFolder) {
            return scopeFolder;
        }
        if (environment) {
            const envFolder = this.workspaceFolderForEnvironment(environment);
            if (envFolder) {
                return envFolder;
            }
        }
        const activeFolder = this.activeWorkspaceFolder();
        if (activeFolder) {
            return activeFolder;
        }
        const folders = this.workspaceFolders();
        if (folders.length === 1) {
            return folders[0];
        }
        return undefined;
    }

    private workspaceFoldersForScope(scope: GetEnvironmentsScope | RefreshEnvironmentsScope): vscode.Uri[] {
        if (scope === 'global') {
            return [];
        }
        if (scope === 'all' || scope === undefined) {
            return this.workspaceFolders();
        }
        if (isUri(scope)) {
            return [this.workspaceFolderForUri(scope) ?? scope];
        }
        return [];
    }

    private cwdForScope(scope: SetEnvironmentScope | GetEnvironmentScope, environment?: PythonEnvironment): string | undefined {
        return this.workspaceFolderForScope(scope, environment)?.fsPath;
    }

    private cachedEnvironmentsForFolders(folders: vscode.Uri[]): PythonEnvironment[] {
        const envs: PythonEnvironment[] = [];
        for (const folder of folders) {
            const cached = this.cachedEnvironmentsByCwd.get(folder.fsPath);
            if (cached) {
                envs.push(...cached);
            }
        }
        return envs;
    }

    private async fetchEnvironmentsForFolders(folders: vscode.Uri[], force = false): Promise<PythonEnvironment[]> {
        if (folders.length === 0) {
            return [];
        }
        const results = await Promise.all(folders.map((folder) => this.fetchEnvironments(folder.fsPath, force)));
        return results.flat();
    }

    private pythonPath(venvPath: string): string {
        const binDir = process.platform === 'win32' ? 'Scripts' : 'bin';
        const pythonBin = process.platform === 'win32' ? 'python.exe' : 'python';
        return path.join(venvPath, binDir, pythonBin);
    }

    private activationPaths(venvPath: string) {
        const binDir = process.platform === 'win32' ? 'Scripts' : 'bin';
        const activateScript = process.platform === 'win32' ? 'activate.bat' : 'activate';
        const activate = path.join(venvPath, binDir, activateScript);
        const deactivate = 'deactivate';
        return { activate, deactivate };
    }

    private logLine(message: string) {
        this.log?.appendLine(`[riot] ${message}`);
    }

    private uniquePkgs(venv: RtVenv): Record<string, string> {
        const diff: Record<string, string> = {};
        for (const [key, value] of Object.entries(venv.pkgs)) {
            if (!(key in venv.shared_pkgs) || venv.shared_pkgs[key] !== value) {
                diff[key] = value;
            }
        }
        return Object.keys(diff).length > 0 ? diff : venv.pkgs;
    }

    private contextEnvDiff(ctx: RtExecutionContext, venv: RtVenv): Record<string, string> {
        const diff: Record<string, string> = {};
        for (const [key, value] of Object.entries(ctx.env)) {
            if (!(key in venv.shared_env) || venv.shared_env[key] !== value) {
                diff[key] = value;
            }
        }
        return diff;
    }

    private normalizePythonVersion(version: string): string {
        const parts = version
            .split('.')
            .map((part) => part.match(/^\d+/)?.[0])
            .filter((part): part is string => Boolean(part))
            .map((part) => Number.parseInt(part, 10))
            .slice(0, 3);

        if (parts.length === 0) {
            return version;
        }

        while (parts.length < 3) {
            parts.push(0);
        }

        return parts.join('.');
    }

    private formatEntries(map: Record<string, string>, maxEntries = 2): string | undefined {
        const entries = Object.entries(map);
        if (entries.length === 0) {
            return undefined;
        }
        entries.sort(([a], [b]) => a.localeCompare(b));
        const shown = entries
            .slice(0, maxEntries)
            .map(([key, value]) => `${key}=${value === '' ? 'latest' : value}`);
        const remaining = entries.length - shown.length;
        const tail = remaining > 0 ? ` +${remaining} more` : '';
        return `${shown.join(', ')}${tail}`;
    }

    private buildDisplayNames(venv: RtVenv, ctx: RtExecutionContext): { displayName: string; shortDisplayName: string } {
        const pkgDetail = this.formatEntries(this.uniquePkgs(venv));
        const envDetail = this.formatEntries(this.contextEnvDiff(ctx, venv));

        const details = [pkgDetail, envDetail].filter((item): item is string => Boolean(item));
        if (details.length === 0) {
            details.push(ctx.hash);
        }

        const separator = ' | ';
        const displayName = `${venv.name} (${venv.python})${separator}${details.join(separator)}`;

        const firstDetail = details[0];
        const shortTail = details.length > 1 ? `${firstDetail} +${details.length - 1} more` : firstDetail;
        const shortDisplayName = `${venv.name} (${venv.python})${separator}${shortTail}`;

        return { displayName, shortDisplayName };
    }

    private indexVenvs(venvs: RtVenv[], cwd: string) {
        const index = new Map<string, RtVenv>();
        for (const venv of venvs) {
            index.set(venv.hash, venv);
            for (const ctx of venv.execution_contexts) {
                index.set(ctx.hash, venv);
            }
        }
        this.venvIndexByCwd.set(cwd, index);
        this.cachedVenvsRawByCwd.set(cwd, venvs);
    }

    private lastEnvKey(cwd: string | undefined): string {
        return `riot:last-env:${cwd ?? '<global>'}`;
    }

    private saveLastEnvironment(cwd: string | undefined, envId: string | undefined): Thenable<void> {
        return this.workspaceState.update(this.lastEnvKey(cwd), envId);
    }

    private getLastEnvironment(cwd: string | undefined): string | undefined {
        return this.workspaceState.get<string>(this.lastEnvKey(cwd));
    }

    private async runRt(args: string[], cwd?: string, signal?: AbortSignal): Promise<string> {
        return new Promise((resolve, reject) => {
            let finished = false;
            let child: cp.ChildProcess | undefined;

            const cleanup = () => {
                if (signal) {
                    signal.removeEventListener('abort', onAbort);
                }
            };

            const onAbort = () => {
                if (finished) {
                    return;
                }
                finished = true;
                cleanup();
                child?.kill();
                reject(new vscode.CancellationError());
            };

            const command = this.rtPath ?? 'rt';
            child = cp.execFile(
                command,
                args,
                { cwd, maxBuffer: MAX_BUFFER_BYTES },
                (err, stdout, stderr) => {
                    if (finished) {
                        return;
                    }
                    finished = true;
                    cleanup();

                    if (stderr?.trim()) {
                        this.logLine(stderr.trim());
                    }

                    if (err) {
                        const message = err instanceof Error ? err.message : String(err);
                        this.logLine(`rt ${args.join(' ')} failed: ${message}`);
                        reject(err);
                        return;
                    }

                    resolve(stdout);
                },
            );

            if (signal) {
                if (signal.aborted) {
                    onAbort();
                } else {
                    signal.addEventListener('abort', onAbort, { once: true });
                }
            }
        });
    }

    buildEnvironment(venv: RtVenv, ctx: RtExecutionContext, workspaceRoot?: string): PythonEnvironment {
        const pythonPath = this.pythonPath(ctx.venv_path);
        const { activate, deactivate } = this.activationPaths(ctx.venv_path);
        const defaultActivation =
            process.platform === 'win32'
                ? [{ executable: activate }]
                : [{ executable: 'source', args: [activate] }];
        const defaultDeactivation =
            process.platform === 'win32'
                ? [{ executable: deactivate }]
                : [{ executable: 'deactivate', args: [] }];
        const displayPath =
            workspaceRoot && path.isAbsolute(ctx.venv_path)
                ? path.relative(workspaceRoot, ctx.venv_path)
                : ctx.venv_path;
        const { displayName, shortDisplayName } = this.buildDisplayNames(venv, ctx);
        if (workspaceRoot) {
            this.envWorkspaceRoots.set(ctx.hash, workspaceRoot);
        }
        return {
            name: venv.name,
            displayName,
            shortDisplayName,
            displayPath,
            version: venv.python,
            environmentPath: vscode.Uri.file(ctx.venv_path),
            execInfo: {
                run: { executable: pythonPath },
                activatedRun: { executable: pythonPath },
                activation: defaultActivation,
                deactivation: defaultDeactivation,
            },
            sysPrefix: ctx.venv_path,
            envId: {
                id: ctx.hash,
                managerId: this.managerId,
            },
        };
    }

    public async fetchVenvByHash(hash: string, cwd?: string): Promise<RtVenv | undefined> {
        if (cwd) {
            const known = this.venvIndexByCwd.get(cwd)?.get(hash);
            if (known) {
                return known;
            }

            await this.fetchEnvironments(cwd, true);
            return this.venvIndexByCwd.get(cwd)?.get(hash);
        }

        for (const index of this.venvIndexByCwd.values()) {
            const known = index.get(hash);
            if (known) {
                return known;
            }
        }

        const mappedRoot = this.envWorkspaceRoots.get(hash);
        if (mappedRoot) {
            await this.fetchEnvironments(mappedRoot, true);
            return this.venvIndexByCwd.get(mappedRoot)?.get(hash);
        }

        const folders = this.workspaceFolders();
        await this.fetchEnvironmentsForFolders(folders, true);
        for (const index of this.venvIndexByCwd.values()) {
            const known = index.get(hash);
            if (known) {
                return known;
            }
        }

        return undefined;
    }

    async getVenvIndexesByWorkspace(): Promise<Map<string, Map<string, RtVenv>>> {
        const folders = this.workspaceFolders();
        await Promise.all(folders.map((folder) => this.fetchEnvironments(folder.fsPath, false)));
        return this.venvIndexByCwd;
    }

    public workspaceUriForEnvironment(environment: PythonEnvironment): vscode.Uri | undefined {
        return this.workspaceFolderForEnvironment(environment);
    }

    private async fetchEnvironments(cwd?: string, force = false): Promise<PythonEnvironment[]> {
        if (!cwd) {
            return [];
        }

        const cached = this.cachedEnvironmentsByCwd.get(cwd);
        if (!force && cached) {
            return cached;
        }
        try {
            const stdout = await this.runRt(['list', '--json'], cwd);
            const parsed = JSON.parse(stdout) as RtVenv[];
            if (!Array.isArray(parsed)) {
                return [];
            }
            for (const venv of parsed) {
                venv.shared_pkgs = venv.shared_pkgs ?? {};
                venv.shared_env = venv.shared_env ?? {};
                venv.python = this.normalizePythonVersion(venv.python);
            }
            this.indexVenvs(parsed, cwd);
            const envs: PythonEnvironment[] = [];
            for (const venv of parsed) {
                if (!Array.isArray(venv.execution_contexts)) {
                    continue;
                }
                for (const ctx of venv.execution_contexts) {
                    envs.push(this.buildEnvironment(venv, ctx, cwd));
                }
            }
            this.cachedEnvironmentsByCwd.set(cwd, envs);
            return envs;
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            this.logLine(`Failed to read environments from rt: ${message}`);
            return cached ?? [];
        }
    }

    private async tryRestoreEnvironment(scope: GetEnvironmentScope | SetEnvironmentScope): Promise<PythonEnvironment | undefined> {
        const cwd = this.cwdForScope(scope);
        if (!cwd) {
            return undefined;
        }
        const savedEnvId = this.getLastEnvironment(cwd);
        if (!savedEnvId) {
            return undefined;
        }

        let encounteredError = false;

        let cached = this.cachedEnvironmentsByCwd.get(cwd);
        if (!cached) {
            cached = await this.fetchEnvironments(cwd);
        }

        let restored = cached.find((env) => env.envId.id === savedEnvId);

        if (!restored) {
            try {
                const venv = await this.fetchVenvByHash(savedEnvId, cwd);
                const ctx = venv?.execution_contexts.find((context) => context.hash === savedEnvId);
                if (venv && ctx) {
                    restored = this.buildEnvironment(venv, ctx, cwd);
                    const updated = this.cachedEnvironmentsByCwd.get(cwd) ?? [];
                    updated.push(restored);
                    this.cachedEnvironmentsByCwd.set(cwd, updated);
                }
            } catch (err) {
                encounteredError = true;
                const message = err instanceof Error ? err.message : String(err);
                this.logLine(`Failed to restore environment ${savedEnvId}: ${message}`);
            }
        }

        if (restored) {
            this.currentEnvironmentByCwd.set(cwd, restored);
            return restored;
        }

        if (!encounteredError) {
            await this.saveLastEnvironment(cwd, undefined);
        }
        return undefined;
    }

    private async setEnvironment(scope: SetEnvironmentScope, environment?: PythonEnvironment): Promise<void> {
        this.log?.info(`Setting environment for scope ${scope ?? '<none>'} to ${environment?.envId.id ?? '<none>'}`);
        const workspaceFolder = this.workspaceFolderForScope(scope, environment);
        const cwd = workspaceFolder?.fsPath;
        if (!cwd) {
            if (environment) {
                void vscode.window.showErrorMessage(
                    'Unable to determine the workspace folder for this Riot environment. Open a file in the target workspace and try again.',
                );
            }
            return;
        }

        const previous = this.currentEnvironmentByCwd.get(cwd);

        const currentCancellation = this.currentSetCancellationByCwd.get(cwd);
        if (currentCancellation) {
            currentCancellation.abort();
            try {
                await this.currentSetPromiseByCwd.get(cwd);
            } catch (err) {
                if (!(err instanceof vscode.CancellationError)) {
                    const message = err instanceof Error ? err.message : String(err);
                    this.logLine(`Previous rt environment change failed: ${message}`);
                }
            }
        }

        if (!environment) {
            await this.updateTestingConfiguration(workspaceFolder, undefined, cwd).catch((err) => {
                const message = err instanceof Error ? err.message : String(err);
                this.logLine(`Failed to clear rt VS Code testing settings: ${message}`);
            });
            await this.saveLastEnvironment(cwd, undefined);
            this.currentEnvironmentByCwd.set(cwd, undefined);
            this.envEmitter.fire({ uri: workspaceFolder, old: previous, new: undefined });
            return;
        }

        const envId = environment.envId.id;
        this.logLine(`Building rt environment ${envId} (cwd=${cwd ?? '<unset>'})`);

        const controller = new AbortController();
        const setPromise = (async () => {
            try {
                await vscode.window.withProgress(
                    {
                        location: vscode.ProgressLocation.Notification,
                        title: `Switching rt environment to ${environment.displayName}`,
                    },
                    async (progress) => {
                        progress.report({ message: `Running rt build ${envId}` });
                        await this.runRt(['build', envId], cwd, controller.signal);
                        progress.report({ message: 'Updating VS Code testing configuration' });
                        await this.updateTestingConfiguration(workspaceFolder, envId, cwd);
                    },
                );
            } catch (err) {
                if (err instanceof vscode.CancellationError) {
                    this.logLine(`rt build ${envId} cancelled.`);
                    return;
                }
                const message = err instanceof Error ? err.message : String(err);
                this.logLine(`rt build ${envId} failed: ${message}`);
                void vscode.window.showErrorMessage(`rt build ${envId} failed: ${message}`);
                throw err;
            } finally {
                if (this.currentSetCancellationByCwd.get(cwd) === controller) {
                    this.currentSetCancellationByCwd.delete(cwd);
                    this.currentSetPromiseByCwd.delete(cwd);
                }
            }
        })();

        this.currentSetCancellationByCwd.set(cwd, controller);
        this.currentSetPromiseByCwd.set(cwd, setPromise);

        await setPromise;

        if (controller.signal.aborted) {
            return;
        }

        this.logLine(`Environment ${envId} set.`);

        await this.saveLastEnvironment(cwd, envId);
        this.currentEnvironmentByCwd.set(cwd, environment);
        this.envEmitter.fire({ uri: workspaceFolder, old: previous, new: environment });
    }

    public async forceReinstallEnvironment(
        scope: GetEnvironmentScope | SetEnvironmentScope,
        environment: PythonEnvironment,
    ): Promise<void> {
        const envId = environment.envId.id;
        const workspaceFolder = this.workspaceFolderForScope(scope, environment);
        const cwd = workspaceFolder?.fsPath ?? environment.environmentPath.fsPath;
        this.logLine(`Force reinstalling rt environment ${envId} (cwd=${cwd ?? '<unset>'})`);

        const controller = new AbortController();

        try {
            await vscode.window.withProgress(
                {
                    location: vscode.ProgressLocation.Notification,
                    title: `Rebuilding rt environment ${environment.displayName}`,
                    cancellable: true,
                },
                async (progress, token) => {
                    token.onCancellationRequested(() => controller.abort());
                    progress.report({ message: `Running rt build ${envId} --force-reinstall` });
                    await this.runRt(['build', envId, '--force-reinstall'], cwd, controller.signal);
                },
            );
        } catch (err) {
            if (err instanceof vscode.CancellationError) {
                this.logLine(`rt build ${envId} --force-reinstall cancelled.`);
                return;
            }
            const message = err instanceof Error ? err.message : String(err);
            this.logLine(`rt build ${envId} --force-reinstall failed: ${message}`);
            void vscode.window.showErrorMessage(`rt build ${envId} --force-reinstall failed: ${message}`);
            throw err;
        }
    }

    private async updateTestingConfiguration(workspaceFolder: vscode.Uri | undefined, envId: string | undefined, cwd?: string): Promise<void> {
        if (!workspaceFolder) {
            return;
        }

        let pytestArgs: string[] | undefined;

        if (envId) {
            const venv = await this.fetchVenvByHash(envId, cwd);
            const ctx = venv?.execution_contexts.find((context) => context.hash === envId);
            const target = ctx?.pytest_target;
            if (target) {
                pytestArgs = [target, '--color=yes', '--cov-branch'];
            }
        }

        const config = vscode.workspace.getConfiguration('python', workspaceFolder);
        const target = vscode.ConfigurationTarget.WorkspaceFolder;

        await Promise.all([
            config.update('testing.pytestArgs', pytestArgs, target),
            config.update('testing.pytestEnabled', pytestArgs ? true : undefined, target),
            config.update('testing.unittestEnabled', pytestArgs ? false : undefined, target),
        ]);
    }

    private async resolveEnvironment(context: ResolveEnvironmentContext): Promise<PythonEnvironment | undefined> {
        const targetPath = context.fsPath;
        const matchesTarget = (env: PythonEnvironment) =>
            env.environmentPath.fsPath === targetPath ||
            env.execInfo.run.executable === targetPath ||
            path.dirname(env.execInfo.run.executable) === targetPath;

        const folder = this.workspaceFolderForUri(context);
        if (folder) {
            if (!this.cachedEnvironmentsByCwd.has(folder.fsPath)) {
                await this.fetchEnvironments(folder.fsPath);
            }
            const envs = this.cachedEnvironmentsByCwd.get(folder.fsPath) ?? [];
            const match = envs.find(matchesTarget);
            if (match) {
                return match;
            }
        }

        if (this.cachedEnvironmentsByCwd.size === 0) {
            const folders = this.workspaceFolders();
            await this.fetchEnvironmentsForFolders(folders);
        }

        for (const envs of this.cachedEnvironmentsByCwd.values()) {
            const match = envs.find(matchesTarget);
            if (match) {
                return match;
            }
        }

        return undefined;
    }
}
