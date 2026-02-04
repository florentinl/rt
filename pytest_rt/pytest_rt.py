"""
Pytest plugin to strip the global Python-version suffix added by tests/conftest.py.

It has two functions:
- restore the original item names and nodeids so that IDE integrations can map collected tests correctly.
- start and stop suitespec services
"""

from time import sleep

import contextlib

import subprocess

import sys
import os
import shlex
from typing import Iterable
from contextlib import suppress

_PY_TAG = f"py{sys.version_info.major}.{sys.version_info.minor}"
_PY_SUFFIX = f"[{_PY_TAG}]"

with suppress(Exception):
    import pytest

    def _strip_suffix(value: str) -> str:
        if value.endswith(_PY_SUFFIX):
            return value[: -len(_PY_SUFFIX)]
        return value

    @pytest.hookimpl(trylast=True)
    def pytest_collection_modifyitems(
        session, config, items: Iterable[pytest.Item]
    ) -> None:
        for item in items:
            item.name = _strip_suffix(item.name)
            # nodeid is stored on the private _nodeid attribute when mutation is needed
            item._nodeid = _strip_suffix(item.nodeid)

    _RIOT_ENTRYPOINT_ENV = "RIOT_ENTRYPOINT_PID"

    def _is_entrypoint() -> bool:
        owner_pid = os.environ.get(_RIOT_ENTRYPOINT_ENV)
        if not owner_pid:
            os.environ[_RIOT_ENTRYPOINT_ENV] = str(os.getpid())
            return True
        return owner_pid == str(os.getpid())

    def pytest_sessionstart(session):
        if not _is_entrypoint():
            return
        suitespec_services = os.getenv("RIOT_SUITESPEC_SERVICES", "")
        if suitespec_services != "":
            print("=== starting services ===")
            project_root = os.getenv("RIOT_PROJECT_ROOT", "")
            services = suitespec_services.split(",")
            subprocess.run(
                ["docker", "compose", "up", "-d", *services], cwd=project_root
            )
            sleep(5)  # Wait a bit for services (postgresql mainly) to be ready

        # Restore service name (otherwise it can be overriden to something like `vscode_pytest`)
        with contextlib.suppress(Exception):
            command_line = os.getenv("RIOT_ORIGINAL_COMMAND")
            if not command_line:
                return
            argv = shlex.split(command_line)

            from ddtrace.internal.settings._inferred_base_service import detect_service
            from ddtrace import config

            config._inferred_base_service = detect_service(argv)
            config.service = detect_service(argv)

    def pytest_sessionfinish(session, exitstatus):
        if not _is_entrypoint():
            return
        suitespec_services = os.getenv("RIOT_SUITESPEC_SERVICES", "")
        if suitespec_services == "":
            return
        project_root = os.getenv("RIOT_PROJECT_ROOT", "")
        services = suitespec_services.split(",")

        print("=== stopping services ===")
        subprocess.run(["docker", "compose", "down", *services], cwd=project_root)
