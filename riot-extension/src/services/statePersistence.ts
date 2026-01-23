/**
 * Service for persisting state to VSCode workspace storage
 * Handles saving and retrieving workspace-specific state
 */

import * as vscode from 'vscode';

export class StatePersistenceService {
    constructor(private readonly workspaceState: vscode.Memento) {}

    /**
     * Save the last selected environment ID for a workspace
     */
    async saveLastEnvironment(workspace: string, envId: string | undefined): Promise<void> {
        const key = this.getKey(workspace, 'last-env');
        await this.workspaceState.update(key, envId);
    }

    /**
     * Get the last selected environment ID for a workspace
     */
    getLastEnvironment(workspace: string): string | undefined {
        const key = this.getKey(workspace, 'last-env');
        return this.workspaceState.get<string>(key);
    }

    /**
     * Clear all saved state for a workspace
     */
    async clearWorkspace(workspace: string): Promise<void> {
        await this.saveLastEnvironment(workspace, undefined);
    }

    /**
     * Clear all saved state
     */
    async clearAll(): Promise<void> {
        const keys = this.workspaceState.keys();
        await Promise.all(
            keys
                .filter((key) => key.startsWith('riot:'))
                .map((key) => this.workspaceState.update(key, undefined)),
        );
    }

    /**
     * Get storage key for a workspace and key name
     */
    private getKey(workspace: string, key: string): string {
        return `riot:${key}:${workspace}`;
    }
}
