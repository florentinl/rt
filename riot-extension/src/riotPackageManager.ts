import * as vscode from 'vscode';
import { Package, PackageManager, PackageManagementOptions, PythonEnvironment } from './api';
import { RiotEnvManager, buildManagerId } from './riotEnvManager';

export class RiotPackageManager implements PackageManager {
    name = 'rt';
    displayName = 'rt';
    description = 'Riot package manager';
    log?: vscode.LogOutputChannel;

    private readonly managerId: string;
    private readonly envManager: RiotEnvManager;
    private readonly pkgEmitter = new vscode.EventEmitter<{
        environment: PythonEnvironment;
        manager: PackageManager;
        changes: { kind: import('./api').PackageChangeKind; pkg: Package }[];
    }>();

    onDidChangePackages = this.pkgEmitter.event;

    constructor(log: vscode.LogOutputChannel, envManager: RiotEnvManager, extensionId: string) {
        this.log = log;
        this.envManager = envManager;
        this.managerId = buildManagerId(extensionId, this.name);
    }

    // Not implemented by request.
    manage(_environment: PythonEnvironment, _options: PackageManagementOptions): Promise<void> {
        return Promise.resolve();
    }

    async refresh(environment: PythonEnvironment): Promise<void> {
        // Simply re-fetch packages; no change events emitted to keep it simple.
        await this.getPackages(environment);
    }

    async getPackages(environment: PythonEnvironment): Promise<Package[] | undefined> {
        try {
            const venvHash = environment.envId.id;
            const scope = this.workspaceUriForEnv(environment);
            const envs = await this.envManager.getEnvironments(scope ?? environment.environmentPath);
            const match = envs.find((env) => env.envId.id === venvHash);
            if (!match) {
                return [];
            }

            const cwd = scope?.fsPath ?? environment.environmentPath.fsPath;
            const venv = await this.envManager.fetchVenvByHash(venvHash, cwd);
            if (!venv) {
                return [];
            }

            return Object.entries(venv.pkgs).map(([name, version]) => ({
                name,
                displayName: name,
                version,
                pkgId: {
                    id: `${name}${version ?? ''}`,
                    managerId: this.managerId,
                    environmentId: venvHash,
                },
            }));
        } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            this.log?.appendLine(`[riot] Failed to list packages: ${msg}`);
            return [];
        }
    }

    private workspaceUriForEnv(environment: PythonEnvironment): vscode.Uri | undefined {
        return this.envManager.workspaceUriForEnvironment(environment);
    }

    clearCache?(): Promise<void> {
        return Promise.resolve();
    }
}
