import * as vscode from "vscode";
import { PythonEnvironments } from "./api";
import { RiotEnvManager } from "./riotEnvManager";
import { RiotPackageManager } from "./riotPackageManager";
import { VenvIndicatorService } from "./services/venvIndicatorService";
import { registerCommands } from "./commands";

// This method is called when your extension is activated
export async function activate(context: vscode.ExtensionContext) {
  const api = await PythonEnvironments.api();
  const extensionId = context.extension.id;

  const log = vscode.window.createOutputChannel("Riot Environment Manager", {
    log: true,
  });
  context.subscriptions.push(log);

  log.appendLine("Riot Environment Manager activating...");
  const envManager = new RiotEnvManager(log, extensionId, context.extensionPath);
  context.subscriptions.push(
    api.registerEnvironmentManager(envManager, { extensionId }),
  );

  const pkgManager = new RiotPackageManager(log, envManager, extensionId);
  context.subscriptions.push(
    api.registerPackageManager(pkgManager, { extensionId }),
  );

  // Set up venv indicator service
  const venvIndicatorService = new VenvIndicatorService(
    api,
    () => envManager.getVenvIndexesByWorkspace(),
    (venv, hash, workspace) => {
      const ctx = venv.execution_contexts.find((c) => c.hash === hash);
      if (!ctx) {
        throw new Error(`Context ${hash} not found in venv ${venv.hash}`);
      }
      return envManager.buildEnvironment(venv, ctx, workspace);
    },
    log,
  );
  context.subscriptions.push(venvIndicatorService);

  const refreshVenvIndicators = vscode.commands.registerCommand(
    "riot.refreshVenvIndicators",
    async () => venvIndicatorService.refresh(),
  );
  context.subscriptions.push(refreshVenvIndicators);

  // Register all commands
  registerCommands(context, api, envManager, extensionId);

  void venvIndicatorService.refresh().catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    log.error(`Failed to refresh Riot file indicators: ${message}`);
  });

  log.appendLine(
    "Riot Environment Manager registered with Python Environments API.",
  );
}
