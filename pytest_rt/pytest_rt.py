"""
Pytest plugin to strip the global Python-version suffix added by tests/conftest.py.

It has two functions:
- restore the original item names and nodeids so that IDE integrations can map collected tests correctly.
- start and stop suitespec services
"""

import subprocess

import sys
import os
from typing import Iterable
from contextlib import suppress

with suppress(Exception):
    import pytest  # type: ignore

    _PY_TAG = f"py{sys.version_info.major}.{sys.version_info.minor}"
    _PY_SUFFIX = f"[{_PY_TAG}]"

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

    _SERVICES_OWNER_ENV = "PYTEST_RT_SERVICES_OWNER_PID"

    def _is_services_owner() -> bool:
        owner_pid = os.environ.get(_SERVICES_OWNER_ENV)
        if not owner_pid:
            os.environ[_SERVICES_OWNER_ENV] = str(os.getpid())
            return True
        return owner_pid == str(os.getpid())

    def pytest_sessionstart(session):
        suitespec_services = os.getenv("SUITESPEC_SERVICES", "")
        if suitespec_services == "":
            return
        if not _is_services_owner():
            return
        project_root = os.getenv("RIOT_PROJECT_ROOT", "")
        services = suitespec_services.split(",")

        print("=== starting services ===")
        subprocess.run(["docker", "compose", "up", "-d", *services], cwd=project_root)

    def pytest_sessionfinish(session, exitstatus):
        suitespec_services = os.getenv("SUITESPEC_SERVICES", "")
        if suitespec_services == "":
            return
        if not _is_services_owner():
            return
        project_root = os.getenv("RIOT_PROJECT_ROOT", "")
        services = suitespec_services.split(",")

        print("=== stopping services ===")
        subprocess.run(["docker", "compose", "down", *services], cwd=project_root)
