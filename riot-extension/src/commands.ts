/**
 * Command handlers for suitespec environments.
 */

import * as vscode from "vscode";
import { PythonEnvironment, PythonEnvironmentApi } from "./api";
import { SuitespecEnvManager } from "./suitespecEnvManager";
import { buildManagerId } from "./types/rtTypes";

/**
 * Register all rt suite commands.
 */
export function registerCommands(
  context: vscode.ExtensionContext,
  api: PythonEnvironmentApi,
  envManager: SuitespecEnvManager,
  extensionId: string,
): void {
  const rtManagerId = buildManagerId(extensionId, "rt");

  // Force reinstall environment command
  const forceReinstallCommand = vscode.commands.registerCommand(
    "riot.forceReinstallEnvironment",
    async () => {
      const scope = await selectWorkspaceFolder();
      if (!scope) {
        void vscode.window.showErrorMessage(
          "No workspace folder selected for suite environment actions.",
        );
        return;
      }

      let environment: PythonEnvironment | undefined;
      try {
        environment = await api.getEnvironment(scope);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        void vscode.window.showErrorMessage(
          `Failed to read selected Python environment: ${message}`,
        );
        return;
      }

      if (!environment) {
        void vscode.window.showErrorMessage(
          "No Python environment selected. Select an rt suite environment and try again.",
        );
        return;
      }

      if (environment.envId.managerId !== rtManagerId) {
        void vscode.window.showErrorMessage(
          "The selected Python environment is not managed by rt. Select a suite environment and try again.",
        );
        return;
      }

      await envManager.forceReinstallEnvironment(scope, environment);
    },
  );
  context.subscriptions.push(forceReinstallCommand);
}

/**
 * Select a workspace folder with fallback logic
 */
async function selectWorkspaceFolder(): Promise<vscode.Uri | undefined> {
  // Try active editor's workspace
  const activeFolder = vscode.window.activeTextEditor
    ? vscode.workspace.getWorkspaceFolder(
        vscode.window.activeTextEditor.document.uri,
      )?.uri
    : undefined;

  if (activeFolder) {
    return activeFolder;
  }

  const workspaceFolders = vscode.workspace.workspaceFolders ?? [];

  // Single workspace
  if (workspaceFolders.length === 1) {
    return workspaceFolders[0].uri;
  }

  // Multiple workspaces: ask user
  if (workspaceFolders.length > 1) {
    const pick = await vscode.window.showQuickPick(
      workspaceFolders.map((folder) => ({
        label: folder.name,
        description: folder.uri.fsPath,
        uri: folder.uri,
      })),
      {
        placeHolder: "Select a workspace folder for the suite environment",
      },
    );
    return pick?.uri;
  }

  return undefined;
}
