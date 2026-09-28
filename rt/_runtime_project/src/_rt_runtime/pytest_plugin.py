"""Pytest integration for an rt-managed suitespec run profile.

It has three functions:
- restore the original item names and nodeids so that IDE integrations can map collected tests correctly.
- start and stop suitespec services, using the same compose project, compose
  files and testagent port discovery as dd-trace-py's scripts/run-tests
- restore the inferred Datadog service from the suitespec run command
"""

import hashlib
import os
import shlex
import subprocess
import sys
from collections.abc import Iterable, Sequence
from contextlib import suppress
from time import sleep

from _rt_runtime import get_context

_RT_CONTEXT = get_context()
_PROJECT_ROOT = _RT_CONTEXT["project_root"]
_SERVICES = _RT_CONTEXT["services"]
_RUN_COMMAND = _RT_CONTEXT["command"]
_PY_TAG = f"py{sys.version_info.major}.{sys.version_info.minor}"
_PY_SUFFIX = f"[{_PY_TAG}]"

# Keep these aligned with scripts/run-tests and scripts/ddtest in dd-trace-py so
# that `rt` and the upstream scripts operate on the very same containers.
_TESTAGENT_PORT_DEFAULT = 9126
_PODMAN_COMPAT_ENV_VAR = "DD_TEST_PODMAN_COMPAT"


def _podman_compat() -> bool:
    return os.environ.get(_PODMAN_COMPAT_ENV_VAR, "0") == "1"


def _ensure_compose_project_name(project_root: str) -> None:
    """Isolate compose projects per checkout, like scripts/run-tests does.

    Without this, compose falls back to the directory name, so `rt` and
    `scripts/run-tests` would manage two different sets of containers competing
    for the same published ports.
    """
    if os.environ.get("COMPOSE_PROJECT_NAME") or not project_root:
        return
    path_hash = hashlib.sha1(project_root.encode()).hexdigest()[:8]
    os.environ["COMPOSE_PROJECT_NAME"] = f"ddtracepy-{path_hash}"


def _compose_command(project_root: str, *args: str) -> list[str]:
    cmd = ["docker", "compose"]
    if _podman_compat():
        cmd += ["-f", os.path.join(project_root, "docker-compose.base.yml")]
        cmd += ["-f", os.path.join(project_root, "docker-compose.podman.yml")]
    else:
        cmd += ["-f", os.path.join(project_root, "docker-compose.yml")]
    cmd += list(args)
    return cmd


def _start_services(project_root: str, services: Sequence[str]) -> None:
    cmd = _compose_command(project_root, "up", "-d", *services)
    if subprocess.run(cmd, cwd=project_root).returncode == 0:
        return

    if "testagent" not in services or _podman_compat():
        return

    # Another checkout already claimed the default testagent port: retry on a
    # random one, same fallback as scripts/run-tests.
    print("=== port conflict, retrying testagent on a random port ===")
    os.environ["TESTAGENT_HOST_PORT"] = "0"
    subprocess.run(
        _compose_command(project_root, "up", "-d", *services), cwd=project_root
    )


def _testagent_url(project_root: str) -> str:
    """Resolve the testagent URL from its actual published host port."""
    if _podman_compat():
        return f"http://localhost:{_TESTAGENT_PORT_DEFAULT}"

    with suppress(Exception):
        result = subprocess.run(
            _compose_command(project_root, "port", "testagent", "8126"),
            cwd=project_root,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0 and result.stdout.strip():
            # e.g. "0.0.0.0:32768" or "127.0.0.1:9126"
            host_port = result.stdout.strip().rsplit(":", 1)[-1]
            return f"http://localhost:{host_port}"

    return f"http://localhost:{_TESTAGENT_PORT_DEFAULT}"


def _stop_services(project_root: str, services: Sequence[str]) -> None:
    # The podman override uses host networking, where compose cannot target
    # individual services on teardown.
    args = ("down",) if _podman_compat() else ("down", *services)
    subprocess.run(_compose_command(project_root, *args), cwd=project_root)


with suppress(Exception):
    import pytest

    def _strip_suffix(value: str) -> str:
        return value.removesuffix(_PY_SUFFIX)

    @pytest.hookimpl(trylast=True)
    def pytest_collection_modifyitems(
        session, config, items: Iterable[pytest.Item]
    ) -> None:
        for item in items:
            item.name = _strip_suffix(item.name)
            # nodeid is stored on the private _nodeid attribute when mutation is needed
            item._nodeid = _strip_suffix(item.nodeid)

    _RT_ENTRYPOINT_ENV = "RT_ENTRYPOINT_PID"

    def _is_entrypoint() -> bool:
        owner_pid = os.environ.get(_RT_ENTRYPOINT_ENV)
        if not owner_pid:
            os.environ[_RT_ENTRYPOINT_ENV] = str(os.getpid())
            return True
        return owner_pid == str(os.getpid())

    def _should_manage_services(session) -> bool:
        return not bool(getattr(session.config.option, "collectonly", False))

    def pytest_sessionstart(session):
        if not _is_entrypoint():
            return
        if _SERVICES and _should_manage_services(session):
            print("=== starting services ===")
            _ensure_compose_project_name(_PROJECT_ROOT)
            _start_services(_PROJECT_ROOT, _SERVICES)
            if "testagent" in _SERVICES:
                url = _testagent_url(_PROJECT_ROOT)
                os.environ["DD_TRACE_AGENT_URL"] = url
                print(f"=== testagent at {url} ===")
            sleep(5)  # Wait a bit for services (postgresql mainly) to be ready

        # Restore service name (otherwise it can be overriden to something like `vscode_pytest`)
        with suppress(Exception):
            if not _RUN_COMMAND:
                return
            argv = shlex.split(_RUN_COMMAND)

            from ddtrace.internal.settings._inferred_base_service import detect_service
            from ddtrace.internal.schema.processor import BaseServiceProcessor
            from ddtrace import config
            from ddtrace import tracer

            original_service = detect_service(argv)

            for processor in tracer._span_aggregator.dd_processors:
                if isinstance(processor, BaseServiceProcessor):
                    processor._global_service = original_service

            config._inferred_base_service = original_service
            config.service = original_service

    def pytest_sessionfinish(session, exitstatus):
        if not _is_entrypoint():
            return
        if not _SERVICES or not _should_manage_services(session):
            return
        _ensure_compose_project_name(_PROJECT_ROOT)

        print("=== stopping services ===")
        _stop_services(_PROJECT_ROOT, _SERVICES)
