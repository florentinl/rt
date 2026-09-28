import { strict as assert } from "node:assert";
import { test } from "node:test";
import { pytestConfigurationArgs } from "../services/pytestConfig";
import { RtTestRun } from "../types/rtTypes";

test("uses suitespec pytest arguments verbatim and excludes cmdargs", () => {
  const run = {
    pytest_args: [
      "--ddtrace",
      "--no-cov",
      "-p",
      "no:flaky",
      "{cmdargs}",
      "tests/ci_visibility",
    ],
  } as RtTestRun;

  assert.deepEqual(pytestConfigurationArgs(run), [
    "--ddtrace",
    "--no-cov",
    "-p",
    "no:flaky",
    "tests/ci_visibility",
  ]);
});

test("clears pytest configuration for non-pytest runs", () => {
  assert.equal(
    pytestConfigurationArgs({ pytest_args: [] } as unknown as RtTestRun),
    undefined,
  );
  assert.equal(pytestConfigurationArgs(undefined), undefined);
});
