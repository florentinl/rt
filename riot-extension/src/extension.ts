// The module 'vscode' contains the VS Code extensibility API
// Import the module and reference it with the alias vscode in your code below
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { PythonEnvironment } from './api';
import { getEnvExtApi } from './pythonEnvsApi';
import { RiotEnvManager, buildManagerId } from './riotEnvManager';
import { RiotPackageManager } from './riotPackageManager';
import { addFileVenvIndicators } from './venvIndicators';

type RtRuntime = {
    rtPath: string;
    binDir: string;
};

const resolveRuntimeTarget = (): string | undefined => {
    if (process.platform === 'darwin' && process.arch === 'arm64') {
        return 'darwin-arm64';
    }

    if (process.platform === 'linux' && process.arch === 'x64') {
        return 'linux-x64';
    }

    return undefined;
};

const fileExists = async (filePath: string): Promise<boolean> => {
    try {
        await fs.promises.access(filePath, fs.constants.F_OK);
        return true;
    } catch {
        return false;
    }
};

const ensureExecutable = async (filePath: string, log: vscode.LogOutputChannel): Promise<void> => {
    try {
        await fs.promises.access(filePath, fs.constants.X_OK);
        return;
    } catch {
        // Fall through to chmod.
    }

    try {
        await fs.promises.chmod(filePath, 0o755);
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        log.appendLine(`Failed to chmod ${filePath}: ${message}`);
    }
};

const resolveRuntimePython = async (
    runtimeDir: string,
): Promise<{ pythonBin: string; binDir: string } | undefined> => {
    const direct = path.join(runtimeDir, 'python', 'bin', 'python');
    if (await fileExists(direct)) {
        return { pythonBin: direct, binDir: path.dirname(direct) };
    }

    const pythonRoot = path.join(runtimeDir, 'python');
    let entries: fs.Dirent[];
    try {
        entries = await fs.promises.readdir(pythonRoot, { withFileTypes: true });
    } catch {
        return undefined;
    }

    for (const entry of entries) {
        if (!entry.isDirectory()) {
            continue;
        }
        const candidate = path.join(pythonRoot, entry.name, 'bin', 'python');
        if (await fileExists(candidate)) {
            return { pythonBin: candidate, binDir: path.dirname(candidate) };
        }
    }

    return undefined;
};

const resolveBundledRuntime = async (
    context: vscode.ExtensionContext,
    log: vscode.LogOutputChannel,
): Promise<RtRuntime | undefined> => {
    if (process.platform === 'win32') {
        return undefined;
    }

    const runtimeRoot = path.join(context.extensionPath, 'runtime');
    const target = resolveRuntimeTarget();
    const candidateDirs: string[] = [];

    if (target) {
        const targetDir = path.join(runtimeRoot, target);
        if (await fileExists(targetDir)) {
            candidateDirs.push(targetDir);
        }
    }

    candidateDirs.push(runtimeRoot);

    let lastMissing: string[] | undefined;
    for (const runtimeDir of candidateDirs) {
        const binDir = path.join(runtimeDir, 'bin');
        const rtPath = path.join(binDir, 'rt');
        const uvPath = path.join(runtimeDir, 'libexec', 'uv');
        const pythonInfo = await resolveRuntimePython(runtimeDir);
        const uvBin = pythonInfo ? path.join(pythonInfo.binDir, 'uv') : undefined;

        const missing: string[] = [];
        if (!(await fileExists(rtPath))) {
            missing.push('rt wrapper');
        }
        if (!(await fileExists(uvPath))) {
            missing.push('uv wrapper');
        }
        if (!pythonInfo) {
            missing.push('python runtime');
        }
        if (!uvBin || !(await fileExists(uvBin))) {
            missing.push('uv binary');
        }

        if (missing.length === 0) {
            await Promise.all([ensureExecutable(rtPath, log), ensureExecutable(uvPath, log)]);
            return { rtPath, binDir };
        }

        lastMissing = missing;
    }

    if (lastMissing && lastMissing.length > 0) {
        log.appendLine(`Bundled runtime not available (${lastMissing.join(', ')}).`);
    }

    return undefined;
};

// This method is called when your extension is activated
// Your extension is activated the very first time the command is executed
export async function activate(context: vscode.ExtensionContext) {
    const api = await getEnvExtApi();
    const extensionId = context.extension.id;
    const riotManagerId = buildManagerId(extensionId, 'riot');

    const log = vscode.window.createOutputChannel('Riot Environment Manager', { log: true });
    context.subscriptions.push(log);

    log.appendLine('Riot Environment Manager activating...');

    const runtime = await resolveBundledRuntime(context, log);
    context.environmentVariableCollection.delete('PATH');
    if (runtime) {
        context.environmentVariableCollection.prepend('PATH', runtime.binDir + path.delimiter);
        log.appendLine(`Using bundled rt from ${runtime.rtPath}`);
    } else {
        log.appendLine('Using rt from PATH.');
    }

    const envManager = new RiotEnvManager(log, extensionId, context.workspaceState, runtime?.rtPath);
    context.subscriptions.push(api.registerEnvironmentManager(envManager));

    const pkgManager = new RiotPackageManager(log, envManager, extensionId);
    context.subscriptions.push(api.registerPackageManager(pkgManager));

    // Set up venv indicators on activation and expose a command to refresh them.
    await addFileVenvIndicators(api, envManager, context, log);
    const refreshVenvIndicators = vscode.commands.registerCommand('riot.refreshVenvIndicators', async () =>
        addFileVenvIndicators(api, envManager, context, log),
    );
    context.subscriptions.push(refreshVenvIndicators);

    const forceReinstallCommand = vscode.commands.registerCommand('riot.forceReinstallEnvironment', async () => {
        const activeFolder = vscode.window.activeTextEditor
            ? vscode.workspace.getWorkspaceFolder(vscode.window.activeTextEditor.document.uri)?.uri
            : undefined;
        const workspaceFolders = vscode.workspace.workspaceFolders ?? [];
        let scope = activeFolder;
        if (!scope) {
            if (workspaceFolders.length === 1) {
                scope = workspaceFolders[0].uri;
            } else if (workspaceFolders.length > 1) {
                const pick = await vscode.window.showQuickPick(
                    workspaceFolders.map((folder) => ({
                        label: folder.name,
                        description: folder.uri.fsPath,
                        uri: folder.uri,
                    })),
                    { placeHolder: 'Select a workspace folder for the Riot environment' },
                );
                scope = pick?.uri;
            }
        }

        if (!scope) {
            void vscode.window.showErrorMessage('No workspace folder selected for Riot environment actions.');
            return;
        }

        let environment: PythonEnvironment | undefined;
        try {
            environment = await api.getEnvironment(scope);
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            void vscode.window.showErrorMessage(`Failed to read selected Python environment: ${message}`);
            return;
        }

        if (!environment) {
            void vscode.window.showErrorMessage(
                'No Python environment selected. Select a Riot-managed environment and try again.',
            );
            return;
        }

        if (environment.envId.managerId !== riotManagerId) {
            void vscode.window.showErrorMessage(
                'The selected Python environment is not managed by Riot. Select a Riot-managed environment and try again.',
            );
            return;
        }

        try {
            await envManager.forceReinstallEnvironment(scope, environment);
        } catch {
            // Errors are surfaced via notifications and the output channel.
        }
    });
    context.subscriptions.push(forceReinstallCommand);

    log.appendLine('Riot Environment Manager registered with Python Environments API.');
}

// This method is called when your extension is deactivated
export function deactivate() { }
