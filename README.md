RT
==

This is a partly vibecoded project that I mostly used to teach myself how PyO3 bindings work and a bit about writing VSCode extension. You probably shouldn't use it. But I think the result of being able to run `riot` tests in the VSCode UI is pretty cool and that is probably achievable using the actual Riot project with a few tweaks.

RT is a vaguely equivalent reimplementation of Datadog's [riot](https://github.com/DataDog/riot), aiming to provide a more familiar command-line interface and better interoperability with other tools.

Why rt ?
--------------------------
The main reasons are:

=> `rt` introduces a venv sub-hash to address two venvs with the same deps but different env vars or commands and allows to run them independently

=> `rt` builds real virtual environments (even with create=False) that you can pass to other tools like IDE extensions or LSPs

Instead of building all nested virtual environments and adding all of their paths to `PYTHONPATH` when running commands, `rt` flattens the venvs and create three directories:
- `venv_self/self_py313`: dd-trace-py install in editable mode
- `venv_deps/venv_{venv_hash}`: installation of all dependencies specified in `riotfile.py`
- `venv_{venv_hash}_{venv_sub_hash}`: actual virtual environment that merges the dependencies from both virtual environments above and injects env variables

Some minor improvements are:
- use `uv` instead of pip and build environments in parallel, so that it is a tad bit faster and python versions are installed lazily by uv
- `rt` caches dev and deps installs by default (in `riot` you have to opt-in with `-s`)
- better shell completion (although it still doesn't work as well as it could)

Install the CLI from Git
------------------------

You can install the Python package directly from the repository using your favorite python package installer

```bash
# uv
uv tool install "git+ssh://git@github.com/florentinl/rt.git"

# pipx
pipx install "git+ssh://git@github.com/florentinl/rt.git"
```

After installation, confirm it is on your PATH:

```bash
rt --help
```

Install the VS Code extension
-----------------------------

1. Download the `.vsix` file from the latest GitHub release of this repository.
2. Install it in VS Code:
   - Via command line: `code --install-extension /path/to/riot-<version>.vsix`
   - Or through the UI: Extensions panel → `...` menu → Install from VSIX.

The extension activates in Riot workspaces (e.g., when `riotfile.py` is present) and pairs with the `rt` CLI to discover environments.
There is no need to install the `rt` cli independently if you only plan to use the vscode extension as it is bundled.
Ensure the Python environments extension is enabled by setting:

```json
"python.useEnvironmentsExtension": true
```

Attributions
-----------------------------
Although there is no shared code, it is heavily based on the original [riot](https://github.com/DataDog/riot)
