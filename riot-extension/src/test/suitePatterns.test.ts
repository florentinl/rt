import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  matchesSuitePattern,
  matchesSuitePatterns,
} from "../services/suitePatterns";

test("matches exact files and directory descendants", () => {
  assert.equal(matchesSuitePattern("tests/unit/test_example.py", "tests/unit"), true);
  assert.equal(matchesSuitePattern("tests/integration/test_example.py", "tests/unit"), false);
  assert.equal(
    matchesSuitePattern("tests/unit/test_example.py", "tests/unit/test_example.py"),
    true,
  );
});

test("matches suitespec glob patterns", () => {
  assert.equal(
    matchesSuitePattern("tests/contrib/flask/test_app.py", "tests/contrib/flask/*"),
    true,
  );
  assert.equal(
    matchesSuitePattern(
      "tests/contrib/flask/nested/test_app.py",
      "tests/contrib/flask/*",
    ),
    true,
  );
  assert.equal(
    matchesSuitePattern("tests/contrib/flask/nested/test_app.py", "tests/**/test_*.py"),
    true,
  );
  assert.equal(
    matchesSuitePattern("tests/contrib/flask/app.py", "tests/**/test_*.py"),
    false,
  );
});

test("normalizes separators and workspace-wide patterns", () => {
  assert.equal(matchesSuitePattern("tests/test_example.py", "."), true);
  assert.equal(
    matchesSuitePattern("tests\\unit\\test_example.py", "tests/unit/*.py"),
    true,
  );
});

test("applies suitespec exclusions after positive matches", () => {
  const patterns = ["ddtrace/appsec/*", "!ddtrace/appsec/ai_guard/*"];
  assert.equal(
    matchesSuitePatterns("ddtrace/appsec/_asm_request_context.py", patterns),
    true,
  );
  assert.equal(
    matchesSuitePatterns("ddtrace/appsec/ai_guard/service.py", patterns),
    false,
  );
  assert.equal(matchesSuitePatterns("tests/appsec/test_context.py", patterns), false);
});
