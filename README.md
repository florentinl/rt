# rt Suite Environments

`rt` makes the test suites declared by dd-trace-py's suitespec available from
the command line and from VS Code's
[Python Environments](https://marketplace.visualstudio.com/items?itemName=ms-python.vscode-python-envs)
UI. Suitespec remains the source of truth for suite names, file patterns,
Python and dependency matrices, run commands, environment variables, and
services. `rt` discovers that normalized model and materializes it with `uv`.

The VS Code extension keeps Microsoft's normal pytest collection, run, debug,
and result flow. It contributes environments and pytest configuration; it does
not implement another test adapter.

## Install the extension

Download the latest `.vsix` from the
[GitHub Releases](https://github.com/florentinl/rt/releases) page, then install
it:

```bash
code --install-extension riot-python-environments-*.vsix
```

Or use **Extensions → ... → Install from VSIX**. Enable the Python
Environments extension in your settings:

```json
"python.useEnvironmentsExtension": true
```

The extension activates for a dd-trace-py checkout containing suitespec files
and `scripts/run-tests`. The VSIX includes the `rt` binary; a separate CLI
installation is not required.

The published extension ID and VSIX filename retain
`florentinl.riot-python-environments` so existing installations and keybindings
upgrade in place. Riot is not used at runtime; all displayed names and behavior
are suitespec-native.

## Install the CLI

To use `rt` from a terminal:

```bash
uv tool install "git+ssh://git@github.com/florentinl/rt.git"
rt --help
```

## Suites, environments, and runs

The model exposed by both the CLI and VS Code is:

```text
Suite                         file patterns, services, suite environment
└── TestEnvironment           Python, dependencies, lockfile, shared venv
    └── TestRun               command and run-specific environment
```

VS Code groups entries by suite. Each concrete `TestEnvironment` owns one
physical environment under `.cache/rt/environments/<hash>`. Its runs reuse that
environment through small launch profiles under
`.cache/rt/profiles/<hash>/<run>/`; the launchers select an immutable external
context with `RT_CONTEXT_PATH`. Selecting one run therefore cannot overwrite
the context used by another run.

Use `--cache-root PATH` before the subcommand to override `.cache/rt`.

Although Riot is no longer used, dd-trace-py currently commits generated
lockfiles under `.riot/requirements`. `rt` reads that upstream-owned legacy
path; it does not use `.riot` as its environment cache or configuration source.

## CLI workflow

List suites and their concrete environments before running a focused subset:

```bash
rt list
rt list flask -p 3.12
rt describe <environment-or-run-id>
rt build flask -p 3.12
rt run flask -p 3.12 -- -vv -k test_request
```

Selectors accept a suite-name pattern, an environment ID, or a run ID.
Arguments after `--` are inserted into the suitespec run command at
`{cmdargs}`. `rt build` and `rt run` materialize environments on demand; use
`--force-reinstall` when a native or dependency change requires a clean build.

## VS Code behavior

### Environment picker and Test Explorer

Choose an entry through **Python: Set Project Environment**. The extension
builds a missing environment on selection and points the Python extension at
the selected run's launcher. It also configures `python.testing.pytestArgs`
from suitespec, so collection, individual test runs, and debugging continue to
use the standard Python Test Explorer.

### Suite code lenses

Test files receive a code lens listing suites whose expanded suitespec patterns
match that file. The quick pick is grouped by suite, then environment and run;
it does not infer ownership by parsing a pytest command.

### Package visibility

The Python Environments view shows packages resolved for the selected
`TestEnvironment`. Dependencies declared by suitespec are marked direct;
packages coming only from the committed lockfile are marked transitive. Suite
environments are immutable from VS Code: update the relevant `suitespec.yml`
and its lockfile, then refresh or rebuild.

### Managed runtime and services

Every physical environment includes `rt-runtime`, which installs the startup
hook, pytest integration, and vendored `envier` typing stubs. For real pytest
sessions the runtime starts services declared by suitespec, discovers the
published testagent port, and stops those services at session end. Collection
only does not start containers, and xdist workers do not duplicate lifecycle
ownership.

## Commands

Open the Command Palette and type **rt**:

| Command | Description |
|---|---|
| **rt: Force Reinstall Selected Environment** | Rebuild the selected shared environment and its run profile. |
| **rt: Refresh Suite Indicators** | Refresh suitespec-backed environments and code lenses. |
| **rt: Select Suite Environment for Current File** | Select among suite runs whose expanded patterns match the active file. |
