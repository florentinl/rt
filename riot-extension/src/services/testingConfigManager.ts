/**
 * Service for managing VSCode testing configuration
 * Handles pytest configuration updates
 */

import * as vscode from 'vscode';
import { RtExecutionContext } from '../types/rtTypes';

export class TestingConfigurationManager {
    constructor(private readonly log: vscode.LogOutputChannel) {}

    /**
     * Update pytest configuration for a workspace folder
     */
    async updateConfiguration(
        workspaceFolder: vscode.Uri,
        context: RtExecutionContext | undefined,
    ): Promise<void> {
        const config = vscode.workspace.getConfiguration('python', workspaceFolder);
        const target = vscode.ConfigurationTarget.WorkspaceFolder;

        const pytestArgs = context?.pytest_target
            ? [context.pytest_target, '--color=yes', '--cov-branch']
            : undefined;

        try {
            await Promise.all([
                config.update('testing.pytestArgs', pytestArgs, target),
                config.update('testing.pytestEnabled', pytestArgs ? true : undefined, target),
                config.update('testing.unittestEnabled', pytestArgs ? false : undefined, target),
            ]);

            if (pytestArgs) {
                this.logLine(`Updated pytest configuration for ${workspaceFolder.fsPath}: ${pytestArgs.join(' ')}`);
            } else {
                this.logLine(`Cleared pytest configuration for ${workspaceFolder.fsPath}`);
            }
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            this.logLine(`Failed to update testing configuration: ${message}`);
            throw err;
        }
    }

    /**
     * Clear testing configuration for a workspace folder
     */
    async clearConfiguration(workspaceFolder: vscode.Uri): Promise<void> {
        await this.updateConfiguration(workspaceFolder, undefined);
    }

    private logLine(message: string): void {
        this.log.appendLine(`[riot] ${message}`);
    }
}
