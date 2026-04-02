/**
 * Service for resolving workspace folders
 * Handles single folder, multi-folder, and out-of-folder file cases
 */

import * as vscode from "vscode";
import {
  GetEnvironmentScope,
  GetEnvironmentsScope,
  PythonEnvironment,
  RefreshEnvironmentsScope,
  SetEnvironmentScope,
} from "../api";

const isUri = (value: unknown): value is vscode.Uri =>
  value instanceof vscode.Uri;

export class WorkspaceResolver {
  /**
   * Get all workspace folders
   */
  getWorkspaceFolders(): vscode.Uri[] {
    return vscode.workspace.workspaceFolders?.map((folder) => folder.uri) ?? [];
  }

  /**
   * Resolve workspace folder from a scope
   * Priority: URI in scope → active editor → single workspace → undefined
   */
  getWorkspaceFolder(
    scope?: SetEnvironmentScope | GetEnvironmentScope | vscode.Uri,
    environment?: PythonEnvironment,
  ): vscode.Uri | undefined {
    // Try to extract URI from scope
    const uri = this.extractUri(scope);
    if (uri) {
      const folder = vscode.workspace.getWorkspaceFolder(uri);
      if (folder) {
        return folder.uri;
      }
      // Out-of-folder file: try other strategies
    }

    // Try environment's path
    if (environment) {
      const folder = vscode.workspace.getWorkspaceFolder(
        environment.environmentPath,
      );
      if (folder) {
        return folder.uri;
      }
    }

    // Use active editor's workspace
    if (vscode.window.activeTextEditor) {
      const folder = vscode.workspace.getWorkspaceFolder(
        vscode.window.activeTextEditor.document.uri,
      );
      if (folder) {
        return folder.uri;
      }
    }

    // Single workspace fallback
    const folders = this.getWorkspaceFolders();
    if (folders.length === 1) {
      return folders[0];
    }

    return undefined;
  }

  /**
   * Resolve multiple workspace folders from a scope
   */
  getWorkspaceFoldersForScope(
    scope: GetEnvironmentsScope | RefreshEnvironmentsScope,
  ): vscode.Uri[] {
    if (scope === "global") {
      return [];
    }
    if (scope === "all" || scope === undefined) {
      return this.getWorkspaceFolders();
    }
    if (isUri(scope)) {
      const folder = this.getWorkspaceFolder(scope);
      return folder ? [folder] : [];
    }
    return [];
  }

  /**
   * Extract URI from various scope types
   */
  private extractUri(
    scope?: SetEnvironmentScope | GetEnvironmentScope | vscode.Uri,
  ): vscode.Uri | undefined {
    if (isUri(scope)) {
      return scope;
    }
    if (Array.isArray(scope) && scope.length > 0 && isUri(scope[0])) {
      return scope[0];
    }
    return undefined;
  }
}
