---
name: run-tests
description: |
  Validate code changes by selecting and running the appropriate test suites
  using the `rt` CLI. Use this when editing code to verify changes work correctly,
  run tests, validate functionality, or check for regressions.
allowed-tools:
  - Bash
  - Read
  - Grep
  - Glob
  - TodoWrite
---

# Test Validation with `rt`

Use the `rt` CLI to run tests. It is a fast Rust-based test environment manager that reads `riotfile.py` and builds isolated venvs using `uv`.

## Quick Reference

```bash
rt list [NAME_PATTERN] [-p PYTHON]             # List venvs
rt run <PATTERN> [-p PYTHON] [-- pytest_args]  # Build + run
rt build <PATTERN> [-p PYTHON]                 # Pre-build only
rt shell <HASH>                                # Interactive shell
rt describe <HASH>                             # Inspect venv config
rt switch <HASH>                               # Link as .venv for IDE
```

## Workflow

### 1. Identify What to Test

Map changed files to suite names:

| Changed path             | Suite name                                 |
| ------------------------ | ------------------------------------------ |
| `ddtrace/_trace/`        | `tracer`                                   |
| `ddtrace/internal/`      | `internal`                                 |
| `ddtrace/contrib/<pkg>/` | `<pkg>` (e.g., `flask`, `django`, `redis`) |
| `ddtrace/appsec/`        | `appsec`                                   |
| `ddtrace/debugging/`     | `debugger`                                 |
| `ddtrace/profiling/`     | `profiling`                                |
| `ddtrace/llmobs/`        | `llmobs`                                   |
| `ddtrace/ci_visibility/` | `ci_visibility`                            |

For contrib integrations, the suite name is typically the package name. When unsure, use `rt list | grep -i <keyword>` to search.

### 2. Find the Right Venv

```bash
rt list flask -p 3.12
```

Output shows a tree with hashes, Python versions, and packages:

```
e06abee flask 3.12 flask~=3.0
└─ e06abee@1a2b3c4  runs: pytest {cmdargs} tests/contrib/flask/
```

- **`e06abee`** — venv hash (use with `rt run` to run all execution context hashes)
- **`e06abee@1a2b3c4`** — execution context hash (also works with `rt run`)

### 3. Run Tests

```bash
# By suite name (regex) + Python version
rt run flask -p 3.12

# By hash
rt run e06abee

# With pytest arguments after --
rt run flask -p 3.12 -- -vv -k test_request

# Multiple suites in parallel
rt run "flask|django" -p 3.12 --parallel

# Run specific test file
rt run e06abee -- tests/contrib/flask/test_views.py -vv
```

### 4. Iterate on Failures

`rt` caches by default — just re-run after fixing code, no flags needed:

```bash
# Re-run specific failing test
rt run flask -p 3.12 -- -vv -k test_failing_case
```

## Selection Rules

1. **Latest Python version by default** — unless targeting specific version compat
2. **One venv per suite is usually enough** — expand only if needed
3. **1-3 venvs total for initial validation** — save broad coverage for CI

## When to Force Reinstall

Use `--force-reinstall` after:

- Modifying `riotfile.py`
- Modifying C/Cython/Rust extensions (`*.pyx`, `*.pxd`, `*.c`, `*.rs`)
- Modifying `setup.py`, `pyproject.toml`, `setup.cfg`, `hatch.toml`
- Merging or rebasing from main

Otherwise just re-run — caching handles it. If native imports fail, you can try rebuilding as well.

## Troubleshooting

```bash
rt list | grep -i <keyword>           # Find suite by name
rt run <pattern> --force-reinstall    # Force clean rebuild
rt clean                              # Remove all cached venvs
```
