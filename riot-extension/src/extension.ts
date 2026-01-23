import * as path from "path";
import * as vscode from "vscode";
import { PythonEnvironment } from "./api";
import { getEnvExtApi } from "./pythonEnvsApi";
import { RiotEnvManager, buildManagerId } from "./riotEnvManager";
import { RiotPackageManager } from "./riotPackageManager";
import { addFileVenvIndicators } from "./venvIndicators";
import { patchPythonEnvironment } from "./patchBundledPython";

// This method is called when your extension is activated
// Your extension is activated the very first time the command is executed
export async function activate(context: vscode.ExtensionContext) {
  const api = await getEnvExtApi();
  const extensionId = context.extension.id;
  const riotManagerId = buildManagerId(extensionId, "riot");

  const log = vscode.window.createOutputChannel("Riot Environment Manager", {
    log: true,
  });
  context.subscriptions.push(log);

  log.appendLine("Riot Environment Manager activating...");

  patchPythonEnvironment(context.extension.extensionPath, log);
  const envManager = new RiotEnvManager(
    log,
    extensionId,
    context.workspaceState,
    path.join(context.extensionPath, "python/bin/rt"),
  );
  context.subscriptions.push(api.registerEnvironmentManager(envManager));

  const pkgManager = new RiotPackageManager(log, envManager, extensionId);
  context.subscriptions.push(api.registerPackageManager(pkgManager));

  // Set up venv indicators on activation and expose a command to refresh them.
  await addFileVenvIndicators(api, envManager, context, log);
  const refreshVenvIndicators = vscode.commands.registerCommand(
    "riot.refreshVenvIndicators",
    async () => addFileVenvIndicators(api, envManager, context, log),
  );
  context.subscriptions.push(refreshVenvIndicators);

  const forceReinstallCommand = vscode.commands.registerCommand(
    "riot.forceReinstallEnvironment",
    async () => {
      const activeFolder = vscode.window.activeTextEditor
        ? vscode.workspace.getWorkspaceFolder(
            vscode.window.activeTextEditor.document.uri,
          )?.uri
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
            {
              placeHolder: "Select a workspace folder for the Riot environment",
            },
          );
          scope = pick?.uri;
        }
      }

      if (!scope) {
        void vscode.window.showErrorMessage(
          "No workspace folder selected for Riot environment actions.",
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
          "No Python environment selected. Select a Riot-managed environment and try again.",
        );
        return;
      }

      if (environment.envId.managerId !== riotManagerId) {
        void vscode.window.showErrorMessage(
          "The selected Python environment is not managed by Riot. Select a Riot-managed environment and try again.",
        );
        return;
      }

      try {
        await envManager.forceReinstallEnvironment(scope, environment);
      } catch {
        // Errors are surfaced via notifications and the output channel.
      }
    },
  );
  context.subscriptions.push(forceReinstallCommand);

  log.appendLine(
    "Riot Environment Manager registered with Python Environments API.",
  );
}

// This method is called when your extension is deactivated
export function deactivate() {}
