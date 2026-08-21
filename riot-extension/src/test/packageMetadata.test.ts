import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  buildPackageMetadata,
  getDirectPackageNames,
  normalizePackageName,
} from "../services/packageMetadata";
import { RtVenv } from "../types/rtTypes";

test("normalizes equivalent Python distribution names", () => {
  assert.equal(normalizePackageName("My_Package.Name"), "my-package-name");
  assert.equal(normalizePackageName("my---package"), "my-package");
});

test("uses Riot declarations to distinguish direct and transitive packages", () => {
  const packages = buildPackageMetadata(
    makeVenv({
      pkgs: { Requests: ">=2" },
      resolved_pkgs: { requests: "2.32.4", urllib3: "2.5.0" },
      display_pkgs: {
        Requests: "2.32.4 (>=2)",
        urllib3: "2.5.0",
      },
    }),
  );

  assert.deepEqual(
    packages.map(({ name, version, isTransitive }) => ({
      name,
      version,
      isTransitive,
    })),
    [
      { name: "Requests", version: "2.32.4", isTransitive: false },
      { name: "urllib3", version: "2.5.0", isTransitive: true },
    ],
  );
  assert.match(packages[0].tooltip, /Declared: Requests>=2/);
  assert.match(packages[0].tooltip, /Installed: 2\.32\.4/);
  assert.deepEqual(getDirectPackageNames(makeVenv({ pkgs: { Requests: ">=2" } })), new Set(["requests"]));
});

test("does not present a constraint as an installed version", () => {
  const [pkg] = buildPackageMetadata(
    makeVenv({
      pkgs: { pytest: ">=8" },
      display_pkgs: { pytest: "(>=8)" },
    }),
  );

  assert.equal(pkg.version, undefined);
  assert.match(pkg.tooltip, /Installed: not resolved/);
});

function makeVenv(overrides: Partial<RtVenv>): RtVenv {
  return {
    hash: "1234567",
    venv_path: "/workspace/.riot/venv_1234567",
    name: "test",
    python: "3.13",
    services: [],
    suite_env: {},
    pkgs: {},
    resolved_pkgs: {},
    display_pkgs: {},
    execution_contexts: [],
    ...overrides,
  };
}
