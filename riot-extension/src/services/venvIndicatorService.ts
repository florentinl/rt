/**
 * Service for managing code lens indicators for test files
 * Shows available Riot environments for test files
 */

import * as path from "path";
import * as vscode from "vscode";
import { PythonEnvironment, PythonEnvironmentApi } from "../api";
import { RtVenv } from "../types/rtTypes";
import { matchesPytestTarget } from "./pytestTargets";

export class VenvIndicatorService {
  private readonly venvsByWorkspace = new Map<string, RtVenv[]>();
  private readonly codeLensEmitter = new vscode.EventEmitter<void>();
  private readonly disposables: vscode.Disposable[] = [];
  private readonly selectCommandId: string;
  private readonly selectActiveCommandId: string;

  constructor(
    private readonly api: PythonEnvironmentApi,
    private readonly getVenvIndexes: () => Promise<
      Map<string, Map<string, RtVenv>>
    >,
    private readonly buildEnvironment: (
      venv: RtVenv,
      hash: string,
      workspace: string,
    ) => PythonEnvironment,
    private readonly log: vscode.LogOutputChannel,
    commandPrefix = "riot",
  ) {
    this.selectCommandId = `${commandPrefix}.selectFileVenv`;
    this.selectActiveCommandId = `${commandPrefix}.selectActiveFileVenv`;
    this.registerCommands();
    this.registerCodeLensProvider();
  }

  /** Refresh the Riot index used to compute indicators for open files. */
  async refresh(): Promise<void> {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    this.venvsByWorkspace.clear();
    if (!workspaceFolders || workspaceFolders.length === 0) {
      this.codeLensEmitter.fire();
      return;
    }

    const venvIndexes = await this.getVenvIndexes();
    for (const folder of workspaceFolders) {
      const index = venvIndexes.get(folder.uri.fsPath);
      if (index && index.size > 0) {
        this.venvsByWorkspace.set(
          folder.uri.fsPath,
          this.normalizeTargets(index),
        );
      }
    }

    this.codeLensEmitter.fire();
    this.log.appendLine("[riot] File venv index refreshed.");
  }

  /**
   * Dispose of all resources
   */
  dispose(): void {
    this.disposables.forEach((d) => d.dispose());
    this.codeLensEmitter.dispose();
  }

  /**
   * Get environments for a file
   */
  getEnvironmentsForFile(filePath: string): PythonEnvironment[] | undefined {
    const uri = vscode.Uri.file(filePath);
    const workspaceFolder = vscode.workspace.getWorkspaceFolder(uri);
    if (!workspaceFolder) {
      return undefined;
    }

    const workspace = workspaceFolder.uri.fsPath;
    const venvs = this.venvsByWorkspace.get(workspace);
    if (!venvs) {
      return undefined;
    }

    const relative = path.relative(workspace, filePath);
    const environments: PythonEnvironment[] = [];
    for (const venv of venvs) {
      for (const context of venv.execution_contexts) {
        if (
          context.pytest_targets.some((target) =>
            matchesPytestTarget(relative, target),
          )
        ) {
          environments.push(
            this.buildEnvironment(venv, context.hash, workspace),
          );
        }
      }
    }
    return environments.length > 0 ? environments : undefined;
  }

  /**
   * Register select environment commands
   */
  private registerCommands(): void {
    // Command to select environment for a specific file
    const selectCommand = vscode.commands.registerCommand(
      this.selectCommandId,
      async (uri: vscode.Uri, envs: PythonEnvironment[]) => {
        if (!envs || envs.length === 0) {
          return;
        }

        const pick = await vscode.window.showQuickPick(
          envs.map((env) => ({
            label: env.displayName,
            description: env.displayPath,
            env,
          })),
          { placeHolder: "Select Riot environment for this file" },
        );

        if (!pick) {
          return;
        }

        const scope = vscode.workspace.getWorkspaceFolder(uri)?.uri ?? uri;
        await this.api.setEnvironment(scope, pick.env);
      },
    );
    this.disposables.push(selectCommand);

    // Command to select environment for the active file
    const selectActiveCommand = vscode.commands.registerCommand(
      this.selectActiveCommandId,
      async () => {
        const uri = vscode.window.activeTextEditor?.document.uri;
        if (!uri) {
          return;
        }

        const envs = this.getEnvironmentsForFile(uri.fsPath);
        if (!envs || envs.length === 0) {
          void vscode.window.showInformationMessage(
            'No Riot environments found for this file. Run "Riot: Refresh Venv Indicators" to refresh.',
          );
          return;
        }

        await vscode.commands.executeCommand(this.selectCommandId, uri, envs);
      },
    );
    this.disposables.push(selectActiveCommand);
  }

  /**
   * Register code lens provider
   */
  private registerCodeLensProvider(): void {
    const provider: vscode.CodeLensProvider = {
      onDidChangeCodeLenses: this.codeLensEmitter.event,
      provideCodeLenses: (document) => {
        const envs = this.getEnvironmentsForFile(document.uri.fsPath);
        if (!envs || envs.length === 0) {
          return [];
        }
        return [
          new vscode.CodeLens(new vscode.Range(0, 0, 0, 0), {
            title: `Select Riot Environment (${envs.length} available)`,
            command: this.selectCommandId,
            arguments: [document.uri, envs],
          }),
        ];
      },
    };

    const registration = vscode.languages.registerCodeLensProvider(
      { language: "python", pattern: "**/tests/**/test_*.py" },
      provider,
    );
    this.disposables.push(registration);
  }

  /**
   * Get unique venvs from an index
   */
  private normalizeTargets(venvIndex: Map<string, RtVenv>): RtVenv[] {
    const unique = new Map<string, RtVenv>();
    for (const venv of venvIndex.values()) {
      unique.set(venv.hash, venv);
    }
    return Array.from(unique.values());
  }

}
