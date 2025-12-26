# Riot Extension For Python Environments

This is a VSCode extension to allow using virtual environments defined using [riot](https://github.com/DataDog/riot) through the [Python Environments Extension](https://marketplace.visualstudio.com/items?itemName=ms-python.vscode-python-envs)

This is a WIP/toy project that you should probably not use, but having it published on the Marketplace simplifies sharing it and demoing it.

## Usage

Enable the [Python Environments Extension](https://marketplace.visualstudio.com/items?itemName=ms-python.vscode-python-envs) with:

```json
"python.useEnvironmentsExtension": true
```

In [dd-trace-py](https://github.com/DataDog/dd-trace-py), select a virtual environment using the command palette action: `Python: Set Project Environment`.

If you need at some point to rebuild native extensions, use: `Riot: Force Reinstall Selected Environment`

From a test file, you can run `Riot: Select Environment for Current File`, to list available virtual environments.
