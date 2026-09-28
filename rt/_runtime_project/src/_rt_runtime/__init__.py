"""Configure an rt-managed suitespec run profile at Python startup."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any

_context: dict[str, Any] | None = None
_context_path: Path | None = None
_CONTEXT_PATH_ENV = "RT_CONTEXT_PATH"


def startup() -> None:
    """Apply the run profile selected by an rt launcher.

    Physical environments are shared by all runs with the same dependency set.
    The launcher selects one immutable external profile through
    ``RT_CONTEXT_PATH`` so run-specific state is never written into that shared
    environment.
    """
    global _context, _context_path

    if _context is not None:
        selected_path = os.environ.get(_CONTEXT_PATH_ENV)
        if selected_path and Path(selected_path).resolve() != _context_path:
            raise RuntimeError("an rt process cannot switch run profiles")
        return

    context_path = os.environ.get(_CONTEXT_PATH_ENV)
    if not context_path:
        return

    manifest_path = Path(context_path).resolve()
    if not manifest_path.is_file():
        raise RuntimeError(
            f"{_CONTEXT_PATH_ENV} does not point to an rt run profile: {manifest_path}"
        )

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
    _context_path = manifest_path


def get_context() -> dict[str, Any]:
    """Return the run profile loaded during interpreter startup."""
    startup()
    if _context is None:
        raise RuntimeError(
            f"{_CONTEXT_PATH_ENV} is not set; run Python through an rt profile launcher"
        )
    return _context
