import path from "path";
import fs from "fs";
import * as vscode from "vscode";

const PLACEHOLDER = "<RT_BUNDLED_PYTHON>";

export function patchPythonEnvironment(
  extensionPath: string,
  log: vscode.LogOutputChannel,
): void {
  const patchedMarker = path.join(extensionPath, ".patched");

  if (fs.existsSync(patchedMarker)) {
    return;
  }

  log.appendLine("Patching Python environment paths...");

  function walkAndPatch(dir: string): void {
    const entries = fs.readdirSync(dir, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walkAndPatch(fullPath);
      } else if (entry.isFile()) {
        try {
          const content = fs.readFileSync(fullPath, "utf8");
          if (content.includes(PLACEHOLDER)) {
            const patched = content.replaceAll(PLACEHOLDER, extensionPath);
            fs.writeFileSync(fullPath, patched, "utf8");
          }
        } catch {
          // Skip binary files that can't be read as text
        }
      }
    }
  }

  if (fs.existsSync(extensionPath)) {
    walkAndPatch(extensionPath);
    fs.writeFileSync(patchedMarker, "", "utf8");
    log.appendLine("Python environment paths patched.");
  }
}
