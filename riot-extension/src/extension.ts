import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import { getEnvExtApi } from "./pythonEnvsApi";
import { RiotEnvManager } from "./riotEnvManager";
import { RiotPackageManager } from "./riotPackageManager";
import { VenvIndicatorService } from "./services/venvIndicatorService";
import { registerCommands } from "./commands";

function resolveBundledRtPath(context: vscode.ExtensionContext): string {
  const platformBinary = process.platform === "win32" ? "rt.exe" : "rt";
  const platformBinaryPath = path.join(context.extensionPath, platformBinary);
  if (fs.existsSync(platformBinaryPath)) {
    return platformBinaryPath;
  }
  return "rt";
}

function ensureRtInPath(
  context: vscode.ExtensionContext,
  rtPath: string,
  log: vscode.LogOutputChannel,
): void {
  const rtDir = path.dirname(rtPath);
  const delimiter = path.delimiter;
  const pathKey =
    Object.keys(process.env).find((key) => key.toLowerCase() === "path") ??
    "PATH";
  const currentPath = process.env[pathKey] ?? "";
  const hasRtDir = currentPath
    .split(delimiter)
    .some((entry) =>
      process.platform === "win32"
        ? entry.toLowerCase() === rtDir.toLowerCase()
        : entry === rtDir,
    );

  if (!hasRtDir) {
    process.env[pathKey] = currentPath
      ? `${rtDir}${delimiter}${currentPath}`
      : rtDir;
    log.appendLine(`[riot] Added ${rtDir} to extension host ${pathKey}.`);
  }

  const mutator = `${rtDir}${delimiter}`;
  context.environmentVariableCollection.persistent = false;
  context.environmentVariableCollection.prepend("PATH", mutator);
  if (process.platform === "win32") {
    context.environmentVariableCollection.prepend("Path", mutator);
  }
  context.environmentVariableCollection.description =
    "Expose bundled rt binary in terminal PATH";
  log.appendLine("[riot] Added bundled rt binary to terminal PATH.");
}

// This method is called when your extension is activated
export async function activate(context: vscode.ExtensionContext) {
  const api = await getEnvExtApi();
  const extensionId = context.extension.id;
  const rtPath = resolveBundledRtPath(context);

  const log = vscode.window.createOutputChannel("Riot Environment Manager", {
    log: true,
  });
  context.subscriptions.push(log);

  log.appendLine("Riot Environment Manager activating...");
  ensureRtInPath(context, rtPath, log);
  const envManager = new RiotEnvManager(
    log,
    extensionId,
    context.workspaceState,
    rtPath,
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
