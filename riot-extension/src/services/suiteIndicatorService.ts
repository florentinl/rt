import * as path from "path";
import * as vscode from "vscode";
import { PythonEnvironment, PythonEnvironmentApi } from "../api";
import { RtSuite, RtTestEnvironment, RtTestRun } from "../types/rtTypes";
import { matchesSuitePatterns } from "./suitePatterns";

export class SuiteIndicatorService implements vscode.Disposable {
  private readonly suitesByWorkspace = new Map<string, RtSuite[]>();
  private readonly codeLensEmitter = new vscode.EventEmitter<void>();
  private readonly disposables: vscode.Disposable[] = [];
  private readonly selectCommandId = "rt.selectFileSuiteEnvironment";
  private readonly selectActiveCommandId = "riot.selectActiveFileVenv";

  constructor(
    private readonly api: PythonEnvironmentApi,
    private readonly getSuites: () => Promise<Map<string, RtSuite[]>>,
    private readonly buildEnvironment: (
      suite: RtSuite,
      environment: RtTestEnvironment,
      run: RtTestRun,
      workspace: string,
    ) => PythonEnvironment,
    private readonly log: vscode.LogOutputChannel,
  ) {
    this.registerCommands();
    this.registerCodeLensProvider();
  }

  async refresh(): Promise<void> {
    this.suitesByWorkspace.clear();
    for (const [workspace, suites] of await this.getSuites()) {
      this.suitesByWorkspace.set(workspace, suites);
    }
    this.codeLensEmitter.fire();
    this.log.appendLine("[rt] Suite pattern index refreshed.");
  }

  dispose(): void {
    this.disposables.forEach((disposable) => disposable.dispose());
    this.codeLensEmitter.dispose();
  }

  getEnvironmentsForFile(filePath: string): PythonEnvironment[] | undefined {
    const uri = vscode.Uri.file(filePath);
    const workspaceFolder = vscode.workspace.getWorkspaceFolder(uri);
    if (!workspaceFolder) {
      return undefined;
    }
    const workspace = workspaceFolder.uri.fsPath;
    const relative = path.relative(workspace, filePath);
    const suites = this.suitesByWorkspace.get(workspace) ?? [];
    const environments: PythonEnvironment[] = [];
    for (const suite of suites) {
      if (!matchesSuitePatterns(relative, suite.patterns)) {
        continue;
      }
      for (const environment of suite.environments) {
        for (const run of environment.runs) {
          environments.push(
            this.buildEnvironment(suite, environment, run, workspace),
          );
        }
      }
    }
    return environments.length > 0 ? environments : undefined;
  }

  private registerCommands(): void {
    this.disposables.push(
      vscode.commands.registerCommand(
        this.selectCommandId,
        async (uri: vscode.Uri, environments: PythonEnvironment[]) => {
          if (!environments?.length) {
            return;
          }
          const pick = await vscode.window.showQuickPick(
            environments.map((environment) => ({
              label: environment.displayName,
              description:
                typeof environment.group === "string"
                  ? environment.group
                  : environment.group?.name,
              detail: environment.displayPath,
              environment,
            })),
            { placeHolder: "Select a suite environment for this file" },
          );
          if (!pick) {
            return;
          }
          const scope = vscode.workspace.getWorkspaceFolder(uri)?.uri ?? uri;
          await this.api.setEnvironment(scope, pick.environment);
        },
      ),
      vscode.commands.registerCommand(this.selectActiveCommandId, async () => {
        const uri = vscode.window.activeTextEditor?.document.uri;
        if (!uri) {
          return;
        }
        const environments = this.getEnvironmentsForFile(uri.fsPath);
        if (!environments?.length) {
          void vscode.window.showInformationMessage(
            'No suitespec environments match this file. Run "rt: Refresh Suite Indicators" to refresh.',
          );
          return;
        }
        await vscode.commands.executeCommand(
          this.selectCommandId,
          uri,
          environments,
        );
      }),
    );
  }

  private registerCodeLensProvider(): void {
    const provider: vscode.CodeLensProvider = {
      onDidChangeCodeLenses: this.codeLensEmitter.event,
      provideCodeLenses: (document) => {
        const environments = this.getEnvironmentsForFile(document.uri.fsPath);
        if (!environments?.length) {
          return [];
        }
        const suites = new Set(
          environments.map((environment) =>
            typeof environment.group === "string"
              ? environment.group
              : environment.group?.name,
          ),
        );
        return [
          new vscode.CodeLens(new vscode.Range(0, 0, 0, 0), {
            title: `Select Suite Environment (${suites.size} suite${suites.size === 1 ? "" : "s"}, ${environments.length} profile${environments.length === 1 ? "" : "s"})`,
            command: this.selectCommandId,
            arguments: [document.uri, environments],
          }),
        ];
      },
    };
    this.disposables.push(
      vscode.languages.registerCodeLensProvider({ language: "python" }, provider),
    );
  }
}
