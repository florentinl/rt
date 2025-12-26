import * as path from 'path';
import * as vscode from 'vscode';
import { PythonEnvironment, PythonEnvironmentApi } from './api';
import { RiotEnvManager, RtVenv } from './riotEnvManager';

const selectCommandId = 'riot.selectFileVenv';
const selectActiveCommandId = 'riot.selectActiveFileVenv';
const ctxForFiles = new Map<string, PythonEnvironment[]>();
const codeLensEmitter = new vscode.EventEmitter<void>();
let codeLensRegistration: vscode.Disposable | undefined;
let selectCommandRegistration: vscode.Disposable | undefined;
let selectActiveCommandRegistration: vscode.Disposable | undefined;

const normalizeTargets = (venvIndex: Map<string, RtVenv>): RtVenv[] => {
    const unique = new Map<string, RtVenv>();
    for (const venv of venvIndex.values()) {
        unique.set(venv.hash, venv);
    }
    return Array.from(unique.values());
};

const matchesTarget = (relativePath: string, target: string): boolean => {
    const rel = path.normalize(relativePath);
    const normalizedTarget = path.normalize(target).replace(new RegExp(`${path.sep}+$`), '');
    if (normalizedTarget === '' || normalizedTarget === '.') {
        return true;
    }
    return rel === normalizedTarget || rel.startsWith(normalizedTarget + path.sep);
};

export const addFileVenvIndicators = async (
    api: PythonEnvironmentApi,
    envManager: RiotEnvManager,
    context: vscode.ExtensionContext,
    log?: vscode.LogOutputChannel,
) => {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders || workspaceFolders.length === 0) {
        return;
    }

    const venvIndexes = await envManager.getVenvIndexesByWorkspace();
    if (venvIndexes.size === 0) {
        return;
    }

    ctxForFiles.clear();

    const venvsByWorkspace = new Map<string, RtVenv[]>();
    for (const folder of workspaceFolders) {
        const index = venvIndexes.get(folder.uri.fsPath);
        if (index && index.size > 0) {
            venvsByWorkspace.set(folder.uri.fsPath, normalizeTargets(index));
        }
    }

    const pattern = '**/tests/**/test_*.py';
    const filesByWorkspace = await Promise.all(
        workspaceFolders.map(async (folder) => ({
            folder,
            files: await vscode.workspace.findFiles(new vscode.RelativePattern(folder, pattern)),
        })),
    );

    for (const { folder, files } of filesByWorkspace) {
        const venvs = venvsByWorkspace.get(folder.uri.fsPath);
        if (!venvs || venvs.length === 0) {
            continue;
        }
        for (const file of files) {
            const workspaceFolder = vscode.workspace.getWorkspaceFolder(file);
            if (!workspaceFolder || workspaceFolder.uri.fsPath !== folder.uri.fsPath) {
                continue;
            }
            const relative = path.relative(folder.uri.fsPath, file.fsPath);
            const envs: PythonEnvironment[] = [];
            for (const venv of venvs) {
                for (const exc of venv.execution_contexts) {
                    const target = exc.pytest_target;
                    if (!target) {
                        continue;
                    }
                    if (matchesTarget(relative, target)) {
                        envs.push(envManager.buildEnvironment(venv, exc, folder.uri.fsPath));
                    }
                }
            }
            if (envs.length > 0) {
                ctxForFiles.set(file.fsPath, envs);
            }
        }
    }

    if (!selectCommandRegistration) {
        selectCommandRegistration = vscode.commands.registerCommand(
            selectCommandId,
            async (uri: vscode.Uri, envs: PythonEnvironment[]) => {
                if (!envs || envs.length === 0) {
                    return;
                }
                const pick = await vscode.window.showQuickPick(
                    envs.map((env) => ({ label: env.displayName, description: env.displayPath, env })),
                    { placeHolder: 'Select Riot environment for this file' },
                );
                if (!pick) {
                    return;
                }
                const scope = vscode.workspace.getWorkspaceFolder(uri)?.uri ?? uri;
                await api.setEnvironment(scope, pick.env);
            },
        );
        context.subscriptions.push(selectCommandRegistration);
    }

    if (!selectActiveCommandRegistration) {
        selectActiveCommandRegistration = vscode.commands.registerCommand(selectActiveCommandId, async () => {
            const uri = vscode.window.activeTextEditor?.document.uri;
            if (!uri) {
                return;
            }
            const envs = ctxForFiles.get(uri.fsPath);
            if (!envs || envs.length === 0) {
                void vscode.window.showInformationMessage('No Riot environments found for this file. Run "Riot: Add Venv Indicators" to refresh.');
                return;
            }
            await vscode.commands.executeCommand(selectCommandId, uri, envs);
        });
        context.subscriptions.push(selectActiveCommandRegistration);
    }

    if (!codeLensRegistration) {
        const provider: vscode.CodeLensProvider = {
            onDidChangeCodeLenses: codeLensEmitter.event,
            provideCodeLenses(document) {
                const envs = ctxForFiles.get(document.uri.fsPath);
                if (!envs || envs.length === 0) {
                    return [];
                }
                return [
                    new vscode.CodeLens(new vscode.Range(0, 0, 0, 0), {
                        title: `Select Riot Environment (${envs.length} available)`,
                        command: selectCommandId,
                        arguments: [document.uri, envs],
                    }),
                ];
            },
        };
        codeLensRegistration = vscode.languages.registerCodeLensProvider({ language: 'python', pattern: '**/tests/**/test_*.py' }, provider);
        context.subscriptions.push(codeLensRegistration);
    }

    codeLensEmitter.fire();
    log?.appendLine('[riot] File venv indicators refreshed.');
};
