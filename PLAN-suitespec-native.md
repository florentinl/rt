# Suitespec-native `rt` migration

## Status

Implemented and validated against the current dd-trace-py suitespec model.
The previous plan predates the removal of `riotfile.py`; this file records the
resulting ownership and compatibility boundaries.

## Ownership

`tests.suitespec` owns suite names, expanded file patterns, matrix variants,
Python versions, dependency declarations, environment hashes, run commands,
run variables, and lockfile locations. `rt` imports that normalized Python API
through PyO3 and exposes a versioned suite-first JSON index. It does not parse
the YAML or reproduce suitespec's matrix and hash rules in Rust.

```text
Suite                         patterns, services, suite environment
└── TestEnvironment           Python, dependencies, lockfile, physical venv
    └── TestRun               command, run variables, immutable profile
```

Each `TestEnvironment` has one physical venv in `.cache/rt/environments`.
Each `TestRun` has a separate manifest and launcher in `.cache/rt/profiles`.
The launcher sets `RT_CONTEXT_PATH`, so selecting a run cannot overwrite state
inside another run's shared interpreter. Profile fingerprints make VS Code
rebuild a profile after its suitespec inputs or resolved packages change.

`rt run` executes the suitespec command from the project root. The runtime
provides pytest integration and service lifecycle. The VS Code extension
exposes one logical Python environment per run, grouped by suite, and sends the
run's pytest arguments to the native Python Test Explorer. Code lenses use the
expanded suite patterns, including exclusions. Direct package metadata comes
from suitespec declarations; installed versions come from the committed lock.

## Compatibility boundaries

- dd-trace-py's generated locks still live under `.riot/requirements`.
  This is an upstream lockfile path, not an rt cache or Riot runtime dependency.
- The extension retains its published ID and three existing `riot.*` command
  IDs so installations and saved keybindings keep working. Its visible names
  and behavior are suitespec-native.
- The `rt list --json` schema is version 1. Changes to the Rust exporter and
  extension consumer must be made together.

## Validation completed

- Rust unit tests, formatting, and Clippy with warnings denied.
- Runtime unit tests and wheel/source distribution build.
- Extension TypeScript, ESLint, unit tests, production bundle, and VS Code
  Electron integration tests.
- Current dd-trace-py export: 180 suites, 2,000 physical environments, and
  2,058 unique run profiles.
- A real Python 3.14 dd-trace-py environment and its run profile were built;
  the smoke test completed through `rt run`.

The full 2,000-environment matrix has not been built. Additional suites should
be exercised through their normal focused and CI workflows as needed.
