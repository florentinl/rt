"""Configure an rt-managed execution context at Python startup."""

import json
import os
import sys
from pathlib import Path

manifest_path = Path(__file__).with_name("rt_context.json")
with manifest_path.open(encoding="utf-8") as manifest_file:
    context = json.load(manifest_file)

project_root = context["project_root"]

if context["no_editable"]:
    # The installed package must take precedence over the local source tree.
    # Keep the project importable for test helpers, but move it behind the
    # installed package.
    sys.path[:] = [path for path in sys.path if path not in ("", project_root)]
    sys.path.append(project_root)

for key, value in context["environment"].items():
    os.environ[key] = value

# Functionally unnecessary, but some dd-trace-py tests require a non-empty
# PYTHONPATH.
os.environ["PYTHONPATH"] = project_root

if "testagent" in context["services"]:
    # pytest_rt refines this from the actual published Compose port.
    os.environ["DD_TRACE_AGENT_URL"] = "http://localhost:9126"

# Pytest reads PYTEST_PLUGINS after sitecustomize runs, so this is equivalent
# to a normal plugin registration without requiring a separate bootstrap file.
plugins = [
    plugin for plugin in os.environ.get("PYTEST_PLUGINS", "").split(",") if plugin
]
if "pytest_rt" not in plugins:
    plugins.append("pytest_rt")
os.environ["PYTEST_PLUGINS"] = ",".join(plugins)
