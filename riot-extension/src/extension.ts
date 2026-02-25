import * as path from "path";
import * as vscode from "vscode";
import { getEnvExtApi } from "./pythonEnvsApi";
import { RiotEnvManager } from "./riotEnvManager";
import { RiotPackageManager } from "./riotPackageManager";
import { VenvIndicatorService } from "./services/venvIndicatorService";
import { registerCommands } from "./commands";

// This method is called when your extension is activated
export async function activate(context: vscode.ExtensionContext) {
  const api = await getEnvExtApi();
  const extensionId = context.extension.id;

  const log = vscode.window.createOutputChannel("Riot Environment Manager", {
    log: true,
  });
  context.subscriptions.push(log);

  log.appendLine("Riot Environment Manager activating...");
  const envManager = new RiotEnvManager(
    log,
    extensionId,
    context.workspaceState,
    path.join(context.extensionPath, "rt"),
  );
  context.subscriptions.push(api.registerEnvironmentManager(envManager));

  const pkgManager = new RiotPackageManager(log, envManager, extensionId);
  context.subscriptions.push(api.registerPackageManager(pkgManager));

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

  await venvIndicatorService.refresh();
  const refreshVenvIndicators = vscode.commands.registerCommand(
    "riot.refreshVenvIndicators",
    async () => venvIndicatorService.refresh(),
  );
  context.subscriptions.push(refreshVenvIndicators);

  // Register all commands
  registerCommands(context, api, envManager, extensionId);

  log.appendLine(
    "Riot Environment Manager registered with Python Environments API.",
  );
}

// This method is called when your extension is deactivated
export function deactivate() {}
