import { strict as assert } from "node:assert";
import * as vscode from "vscode";
import { PackageManagementOptions, PythonEnvironment } from "../api";
import { RiotEnvManager } from "../riotEnvManager";
import { RiotPackageManager } from "../riotPackageManager";

suite("Riot extension", () => {
  test("activates on VS Code 1.134", async () => {
    const extension = vscode.extensions.getExtension(
      "florentinl.riot-python-environments",
    );
    assert.ok(extension, "Riot extension is available in the extension host");

    await extension.activate();
    assert.equal(extension.isActive, true);
  });

  test("rejects package mutations for immutable Riot environments", async () => {
    const manager = new RiotPackageManager(
      {} as vscode.LogOutputChannel,
      {} as RiotEnvManager,
      "florentinl.riot-python-environments",
    );

    await assert.rejects(
      manager.manage({} as PythonEnvironment, {
        install: ["requests"],
      } as PackageManagementOptions),
      (error: Error) =>
        error.name === "RiotEnvironmentImmutableError" &&
        error.message.includes("Edit riotfile.py"),
    );
  });
});
