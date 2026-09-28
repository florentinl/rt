import { strict as assert } from "node:assert";
import * as vscode from "vscode";
import { PackageManagementOptions, PythonEnvironment } from "../api";
import { SuitespecEnvManager } from "../suitespecEnvManager";
import { SuitespecPackageManager } from "../suitespecPackageManager";
import { RtSuite, RtTestEnvironment, RtTestRun } from "../types/rtTypes";

suite("rt suite extension", () => {
  test("activates on VS Code 1.134", async () => {
    const extension = vscode.extensions.getExtension(
      "florentinl.riot-python-environments",
    );
    assert.ok(extension, "rt suite extension is available in the extension host");

    await extension.activate();
    assert.equal(extension.isActive, true);
  });

  test("rejects package mutations for immutable suite environments", async () => {
    const manager = new SuitespecPackageManager(
      {} as vscode.LogOutputChannel,
      {} as SuitespecEnvManager,
      "florentinl.riot-python-environments",
    );

    await assert.rejects(
      manager.manage({} as PythonEnvironment, {
        install: ["requests"],
      } as PackageManagementOptions),
      (error: Error) =>
        error.name === "SuitespecEnvironmentImmutableError" &&
        error.message.includes("Edit suitespec.yml"),
    );
  });

  test("exposes each run as a suite-grouped logical Python environment", () => {
    const log = {
      appendLine: () => undefined,
      error: () => undefined,
      info: () => undefined,
    } as unknown as vscode.LogOutputChannel;
    const manager = new SuitespecEnvManager(
      log,
      "florentinl.riot-python-environments",
    );
    const suite = makeSuite();
    const dependencyEnvironment = suite.environments[0];
    const first = manager.buildEnvironment(
      suite,
      dependencyEnvironment,
      dependencyEnvironment.runs[0],
      "/workspace",
    );
    const second = manager.buildEnvironment(
      suite,
      dependencyEnvironment,
      dependencyEnvironment.runs[1],
      "/workspace",
    );

    assert.notEqual(first.envId.id, second.envId.id);
    assert.notEqual(first.environmentPath.fsPath, second.environmentPath.fsPath);
    assert.equal(first.sysPrefix, dependencyEnvironment.venv_path);
    assert.equal(first.execInfo.run.executable, dependencyEnvironment.runs[0].python_path);
    assert.equal(
      typeof first.group === "string" ? first.group : first.group?.name,
      suite.id,
    );
    assert.match(String(first.tooltip), /Direct dependencies: pytest>=8/);
    assert.match(String(first.tooltip), /Command: pytest tests\/unit\/test_0\.py/);
    assert.match(String(first.tooltip), /Differing run environment: RUN_INDEX=0/);
    assert.doesNotMatch(String(first.tooltip), /SHARED/);
    assert.doesNotMatch(String(first.tooltip), /Run: 1/);
    assert.match(String(first.tooltip), /Profile:/);
  });
});

function makeSuite(): RtSuite {
  const environment: RtTestEnvironment = {
    hash: "abc1234",
    name: "unit-py313",
    display_name: "unit-py313 · Python 3.13",
    integration_name: "unit",
    python: "3.13",
    direct_dependencies: ["pytest>=8"],
    resolved_pkgs: { pytest: "8.4.2" },
    display_pkgs: { pytest: "8.4.2" },
    lockfile: "/workspace/.uv/locks/abc1234.txt",
    venv_path: "/workspace/.cache/rt/environments/abc1234",
    python_path: "/workspace/.cache/rt/environments/abc1234/bin/python",
    activate_path: "/workspace/.cache/rt/environments/abc1234/bin/activate",
    runs: [makeRun(0), makeRun(1)],
  };
  return {
    id: "unit",
    name: "unit",
    patterns: ["tests/unit/**"],
    source_files: ["tests/suitespec.yml"],
    services: ["testagent"],
    suite_env: {},
    environments: [environment],
  };
}

function makeRun(index: number): RtTestRun {
  const base = `/workspace/.cache/rt/profiles/abc1234/${index}`;
  return {
    id: `abc1234@${index}`,
    index,
    command: `pytest tests/unit/test_${index}.py`,
    env: { SHARED: "fixed", RUN_INDEX: String(index) },
    differing_env: { RUN_INDEX: String(index) },
    pytest_args: [`tests/unit/test_${index}.py`],
    profile_fingerprint: `fingerprint-${index}`,
    pytest_targets: [`tests/unit/test_${index}.py`],
    profile_path: `${base}/profile.json`,
    display_name: `unit-py313 · Python 3.13 · RUN_INDEX=${index}`,
    short_display_name: `unit-py313 (3.13) · RUN_INDEX=${index}`,
    venv_path: base,
    python_path: `${base}/bin/python`,
    activate_path: `${base}/bin/activate`,
  };
}
