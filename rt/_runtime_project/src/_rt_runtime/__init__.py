"""Configure an rt-managed execution context at Python startup."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any

_context: dict[str, Any] | None = None


def _configure_project_path(context: dict[str, Any]) -> None:
    if not context["no_editable"]:
        return

    project_root = context["project_root"]
    # The installed package must take precedence over the local source tree.
    # Keep the project importable for test helpers, but move it behind the
    # installed package.
    sys.path[:] = [path for path in sys.path if path not in ("", project_root)]
    sys.path.append(project_root)


def startup() -> None:
    """Apply the execution context recorded by rt for this virtual environment."""
    global _context

    if _context is not None:
        return

    manifest_path = Path(sys.prefix) / "rt_config.json"
    if not manifest_path.is_file():
        return

    with manifest_path.open(encoding="utf-8") as manifest_file:
        context = json.load(manifest_file)

    project_root = context["project_root"]

    _configure_project_path(context)

    for key, value in context["environment"].items():
        os.environ[key] = value

    # Functionally unnecessary, but some dd-trace-py tests require a non-empty
    # PYTHONPATH.
    os.environ["PYTHONPATH"] = project_root

    if "testagent" in context["services"]:
        # The pytest plugin refines this from the actual published Compose port.
        os.environ["DD_TRACE_AGENT_URL"] = "http://localhost:9126"

    plugins = [
        plugin
        for plugin in os.environ.get("PYTEST_PLUGINS", "").split(",")
        if plugin
    ]
    if "_rt_runtime.pytest_plugin" not in plugins:
        plugins.append("_rt_runtime.pytest_plugin")
    os.environ["PYTEST_PLUGINS"] = ",".join(plugins)

    _context = context


def get_context() -> dict[str, Any]:
    """Return the execution context loaded during interpreter startup."""
    startup()
    if _context is None:
        raise RuntimeError("rt_config.json is missing from this virtual environment")
    return _context
