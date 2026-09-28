import * as vscode from "vscode";
import { PythonEnvironments } from "./api";
import { SuitespecEnvManager } from "./suitespecEnvManager";
import { SuitespecPackageManager } from "./suitespecPackageManager";
import { SuiteIndicatorService } from "./services/suiteIndicatorService";
import { registerCommands } from "./commands";

// This method is called when your extension is activated
export async function activate(context: vscode.ExtensionContext) {
  const api = await PythonEnvironments.api();
  const extensionId = context.extension.id;

  const log = vscode.window.createOutputChannel("rt Suite Environments", {
    log: true,
  });
  context.subscriptions.push(log);

  log.appendLine("rt suite environment manager activating...");
  const envManager = new SuitespecEnvManager(log, extensionId, context.extensionPath);
  context.subscriptions.push(
    api.registerEnvironmentManager(envManager, { extensionId }),
  );

  const pkgManager = new SuitespecPackageManager(log, envManager, extensionId);
  context.subscriptions.push(
    api.registerPackageManager(pkgManager, { extensionId }),
  );

  const suiteIndicatorService = new SuiteIndicatorService(
    api,
    () => envManager.getSuitesByWorkspace(),
    (suite, environment, run, workspace) =>
      envManager.buildEnvironment(suite, environment, run, workspace),
    log,
  );
  context.subscriptions.push(suiteIndicatorService);

  const refreshSuiteIndicators = vscode.commands.registerCommand(
    "riot.refreshVenvIndicators",
    async () => suiteIndicatorService.refresh(),
  );
  context.subscriptions.push(refreshSuiteIndicators);

  // Register all commands
  registerCommands(context, api, envManager, extensionId);

  void suiteIndicatorService.refresh().catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    log.error(`Failed to refresh suitespec file indicators: ${message}`);
  });

  log.appendLine(
    "rt suite environment manager registered with Python Environments API.",
  );
}
