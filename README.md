# Riot Python Environments

A VS Code extension that brings [riot](https://github.com/DataDog/riot) virtual environments into the [Python Environments](https://marketplace.visualstudio.com/items?itemName=ms-python.vscode-python-envs) UI. Select a riot environment, run tests from the sidebar, get per-file code lenses, and let the extension handle service containers for you.



https://github.com/user-attachments/assets/b3920bbc-c7b5-4c14-8fdc-4f067fcdd777



## Getting Started

### Install the Extension

Download the latest `.vsix` from the [GitHub Releases](https://github.com/florentinl/rt/releases) page, then install it:

```bash
code --install-extension riot-python-environments-*.vsix
```

Or through the UI: Extensions panel → `...` menu → **Install from VSIX**.

Then enable the Python Environments extension in your settings:

```json
"python.useEnvironmentsExtension": true
```

The extension activates automatically when a `riotfile.py` is detected in your workspace.

The VSIX ships with its own bundled `rt` runtime, so no separate install is needed to use the extension.

### (Optional) Install the CLI

If you also want to run `rt` from your terminal, install it with your preferred tool manager:

```bash
# uv (recommended)
uv tool install "git+ssh://git@github.com/florentinl/rt.git"

# pipx
pipx install "git+ssh://git@github.com/florentinl/rt.git"
```

Verify it works:

```bash
rt --help
```

## Features

### Environment Picker

Select any riot environment through `Python: Set Project Environment`. The extension discovers every venv defined in your riotfile, builds it on demand, and wires it into VS Code as the active interpreter.

### Test UI Integration

When you select an environment, the extension configures `python.testing.pytestArgs` automatically so the VS Code test sidebar picks up the right pytest target. Run, debug, and re-run tests without leaving the editor.

### Code Lenses

Test files (`tests/**/test_*.py`) get a clickable code lens allowing to select the riot environments that apply. Click it to quick-pick an environment scoped to that file.

### Package Visibility

The Python Environments view shows the installed version of every package in a
Riot environment. Packages declared in `riotfile.py` are shown as direct
dependencies, while packages coming from the resolved lockfile are marked as
transitive. Riot environments remain read-only in VS Code: edit `riotfile.py`
and refresh or rebuild instead of installing or removing packages in place.

### Managed Runtime

Every generated environment includes the bundled `rt-runtime` package. It owns
the Python startup hook, pytest integration, and typing stubs; uv builds it once
and reuses the cached wheel across compatible environments. The only context
file written by rt is `<venv>/rt_config.json`, which also records the rt version
used to complete the environment.

### Typed `envier` Settings

`envier` ships no `py.typed` marker, so every setting declared through it shows
up as untyped in the editor. The `rt-runtime` package installs a vendored
`types-envier` stub package in every managed environment - class access resolves
to `EnvVariable[T]`, instance access to `T`. See
`rt/_runtime_project/vendor/types-envier/VENDORING.md`.

### Service Containers

Environments that declare services in `tests/suitespec.py` (including the `testagent` snapshot service) are wired up automatically. The extension injects a pytest plugin that starts and stops the right containers around your test run.

## Commands

Open the Command Palette (`Cmd+Shift+P` / `Ctrl+Shift+P`) and type **Riot** to see:

| Command | Description |
|---|---|
| **Riot: Force Reinstall Selected Environment** | Tears down and rebuilds the currently selected environment from scratch. Useful after changing native dependencies or when the venv is in a bad state. |
| **Riot: Refresh Riot Venv Indicators** | Refreshes the Riot environment index used by code lenses. Run this after editing `riotfile.py`. |
| **Riot: Select Riot Environment for Current File** | Shows a quick-pick of environments whose pytest target matches the file you have open. |
