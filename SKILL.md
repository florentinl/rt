---
name: run-tests
description: |
  Validate dd-trace-py changes by selecting and running suitespec-defined test
  suites with the rt CLI. Use this for focused tests, regression checks, and
  test-environment inspection.
allowed-tools:
  - Bash
  - Read
  - Grep
  - Glob
  - TodoWrite
---

# Test validation with `rt`

`rt` consumes dd-trace-py's normalized suitespec model and materializes its
`TestEnvironment` entries with `uv`. Suitespec owns suite selection, matrices,
dependencies, commands, environment variables, file patterns, and services.

## Quick reference

```bash
rt list [SUITE_PATTERN] [-p PYTHON]
rt describe <ENVIRONMENT_OR_RUN_ID>
rt build <SUITE_PATTERN_OR_ID> [-p PYTHON]
rt run <SUITE_PATTERN_OR_ID> [-p PYTHON] [-- PYTEST_ARGS]
```

The default cache is `<project>/.cache/rt`. Override it with
`rt --cache-root PATH <command>`. Physical environments are keyed by
`TestEnvironment`; multiple `TestRun` profiles can safely share one environment.

## Workflow

### 1. Find the owning suite

Start from the changed path, then query suite names and file-pattern ownership:

```bash
rt list flask
rt list --test tests/contrib/flask/test_views.py
```

Do not derive suite ownership from a run's pytest command. `rt` uses the
expanded patterns exported by suitespec.

### 2. Inspect the matrix

Narrow to a Python version and inspect an exact environment or run before
launching a broad matrix:

```bash
rt list flask -p 3.12
rt describe <environment-or-run-id>
```

A suite contains one or more dependency environments, and an environment can
contain multiple commands or environment variants. Prefer an exact run ID when
only one profile is relevant.

### 3. Run focused tests

```bash
# Suite pattern and Python version
rt run flask -p 3.12

# Exact environment or run
rt run <id>

# Arguments forwarded at the suitespec command's {cmdargs} placeholder
rt run flask -p 3.12 -- -vv -k test_request

# Multiple matching suites/runs
rt run "flask|django" -p 3.12 --parallel

# Select by test ownership
rt run flask -p 3.12 --test tests/contrib/flask/test_views.py -- -vv
```

### 4. Interpret and iterate

Report test assertion results separately from environment, Docker, or service
harness failures. A setup failure is not a passing or failing test result.
After a source-only change, rerun normally; the shared environment remains
valid.

## When to force reinstall

Use `--force-reinstall` after changes that invalidate a materialized package or
dependency environment, including:

- a suitespec dependency or committed lockfile change;
- C, Cython, Rust, build configuration, or packaging changes;
- a rebase that changes native build inputs;
- a stale or broken installed artifact.

```bash
rt run flask -p 3.12 --force-reinstall -- -vv -k test_request
```

Do not force reinstall for ordinary pure-Python editable source changes.

## Ownership and cache notes

- `.cache/rt/environments` contains shared physical environments.
- `.cache/rt/profiles` contains immutable per-run manifests and launchers.
- `RT_CONTEXT_PATH` selects a run profile; do not write context into a shared
  environment.
- dd-trace-py's committed locks still live under `.riot/requirements` for
  upstream compatibility. Riot is not an `rt` dependency or source of truth.

## Troubleshooting

```bash
rt list <keyword>
rt describe <id>
rt run <id> --force-reinstall
rt clean
```

Use the repository's broader validation and lint commands when the change
requires more than the focused suitespec run.
