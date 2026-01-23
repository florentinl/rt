/**
 * Service for managing code lens indicators for test files
 * Shows available Riot environments for test files
 */

import * as path from 'path';
import * as vscode from 'vscode';
import { PythonEnvironment, PythonEnvironmentApi } from '../api';
import { RtVenv } from '../types/rtTypes';

export class VenvIndicatorService {
    private readonly ctxForFiles = new Map<string, PythonEnvironment[]>();
    private readonly codeLensEmitter = new vscode.EventEmitter<void>();
    private readonly disposables: vscode.Disposable[] = [];
    private readonly selectCommandId: string;
    private readonly selectActiveCommandId: string;

    constructor(
        private readonly api: PythonEnvironmentApi,
        private readonly getVenvIndexes: () => Promise<Map<string, Map<string, RtVenv>>>,
        private readonly buildEnvironment: (venv: RtVenv, hash: string, workspace: string) => PythonEnvironment,
        private readonly log: vscode.LogOutputChannel,
        commandPrefix = 'riot',
    ) {
        this.selectCommandId = `${commandPrefix}.selectFileVenv`;
        this.selectActiveCommandId = `${commandPrefix}.selectActiveFileVenv`;
        this.registerCommands();
        this.registerCodeLensProvider();
    }

    /**
     * Refresh indicators by scanning for test files and matching environments
     */
    async refresh(): Promise<void> {
        const workspaceFolders = vscode.workspace.workspaceFolders;
        if (!workspaceFolders || workspaceFolders.length === 0) {
            return;
        }

        const venvIndexes = await this.getVenvIndexes();
        if (venvIndexes.size === 0) {
            return;
        }

        this.ctxForFiles.clear();

        // Get unique venvs per workspace
        const venvsByWorkspace = new Map<string, RtVenv[]>();
        for (const folder of workspaceFolders) {
            const index = venvIndexes.get(folder.uri.fsPath);
            if (index && index.size > 0) {
                venvsByWorkspace.set(folder.uri.fsPath, this.normalizeTargets(index));
            }
        }

        // Find all test files matching the pattern
        const pattern = '**/tests/**/test_*.py';
        const filesByWorkspace = await Promise.all(
            workspaceFolders.map(async (folder) => ({
                folder,
                files: await vscode.workspace.findFiles(new vscode.RelativePattern(folder, pattern)),
            })),
        );

        // Match test files to execution contexts
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
                        if (this.matchesTarget(relative, target)) {
                            envs.push(this.buildEnvironment(venv, exc.hash, folder.uri.fsPath));
                        }
                    }
                }

                if (envs.length > 0) {
                    this.ctxForFiles.set(file.fsPath, envs);
                }
            }
        }

        this.codeLensEmitter.fire();
        this.log.appendLine('[riot] File venv indicators refreshed.');
    }

    /**
     * Dispose of all resources
     */
    dispose(): void {
        this.disposables.forEach((d) => d.dispose());
        this.codeLensEmitter.dispose();
    }

    /**
     * Get environments for a file
     */
    getEnvironmentsForFile(filePath: string): PythonEnvironment[] | undefined {
        return this.ctxForFiles.get(filePath);
    }

    /**
     * Register select environment commands
     */
    private registerCommands(): void {
        // Command to select environment for a specific file
        const selectCommand = vscode.commands.registerCommand(
            this.selectCommandId,
            async (uri: vscode.Uri, envs: PythonEnvironment[]) => {
                if (!envs || envs.length === 0) {
                    return;
                }

                const pick = await vscode.window.showQuickPick(
                    envs.map((env) => ({
                        label: env.displayName,
                        description: env.displayPath,
                        env,
                    })),
                    { placeHolder: 'Select Riot environment for this file' },
                );

                if (!pick) {
                    return;
                }

                const scope = vscode.workspace.getWorkspaceFolder(uri)?.uri ?? uri;
                await this.api.setEnvironment(scope, pick.env);
            },
        );
        this.disposables.push(selectCommand);

        // Command to select environment for the active file
        const selectActiveCommand = vscode.commands.registerCommand(
            this.selectActiveCommandId,
            async () => {
                const uri = vscode.window.activeTextEditor?.document.uri;
                if (!uri) {
                    return;
                }

                const envs = this.ctxForFiles.get(uri.fsPath);
                if (!envs || envs.length === 0) {
                    void vscode.window.showInformationMessage(
                        'No Riot environments found for this file. Run "Riot: Refresh Venv Indicators" to refresh.',
                    );
                    return;
                }

                await vscode.commands.executeCommand(this.selectCommandId, uri, envs);
            },
        );
        this.disposables.push(selectActiveCommand);
    }

    /**
     * Register code lens provider
     */
    private registerCodeLensProvider(): void {
        const provider: vscode.CodeLensProvider = {
            onDidChangeCodeLenses: this.codeLensEmitter.event,
            provideCodeLenses: (document) => {
                const envs = this.ctxForFiles.get(document.uri.fsPath);
                if (!envs || envs.length === 0) {
                    return [];
                }
                return [
                    new vscode.CodeLens(new vscode.Range(0, 0, 0, 0), {
                        title: `Select Riot Environment (${envs.length} available)`,
                        command: this.selectCommandId,
                        arguments: [document.uri, envs],
                    }),
                ];
            },
        };

        const registration = vscode.languages.registerCodeLensProvider(
            { language: 'python', pattern: '**/tests/**/test_*.py' },
            provider,
        );
        this.disposables.push(registration);
    }

    /**
     * Get unique venvs from an index
     */
    private normalizeTargets(venvIndex: Map<string, RtVenv>): RtVenv[] {
        const unique = new Map<string, RtVenv>();
        for (const venv of venvIndex.values()) {
            unique.set(venv.hash, venv);
        }
        return Array.from(unique.values());
    }

    /**
     * Check if a file path matches a pytest target
     */
    private matchesTarget(relativePath: string, target: string): boolean {
        const rel = path.normalize(relativePath);
        const normalizedTarget = path.normalize(target).replace(new RegExp(`${path.sep}+$`), '');
        if (normalizedTarget === '' || normalizedTarget === '.') {
            return true;
        }
        return rel === normalizedTarget || rel.startsWith(normalizedTarget + path.sep);
    }
}
