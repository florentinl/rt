import { strict as assert } from "node:assert";
import { test } from "node:test";
import { matchesPytestTarget } from "../services/pytestTargets";

test("matches files below a pytest directory target", () => {
  assert.equal(
    matchesPytestTarget("tests/unit/test_example.py", "tests/unit/"),
    true,
  );
  assert.equal(
    matchesPytestTarget("tests/integration/test_example.py", "tests/unit/"),
    false,
  );
});

test("matches a pytest file target with a node id", () => {
  assert.equal(
    matchesPytestTarget(
      "tests/unit/test_example.py",
      "tests/unit/test_example.py::TestExample::test_it",
    ),
    true,
  );
});

test("matches a workspace-wide pytest target", () => {
  assert.equal(matchesPytestTarget("tests/test_example.py", "."), true);
});
