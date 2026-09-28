from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import _rt_runtime
from _rt_runtime import get_context, startup


class RuntimeProfileTest(unittest.TestCase):
    def tearDown(self) -> None:
        _rt_runtime._context = None
        _rt_runtime._context_path = None

    def test_startup_requires_an_explicit_profile(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            startup()

            with self.assertRaisesRegex(RuntimeError, "RT_CONTEXT_PATH is not set"):
                get_context()

    def test_startup_does_not_read_a_manifest_from_the_shared_environment(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            legacy_path = Path(temporary_directory) / "rt_config.json"
            legacy_path.write_text("{}", encoding="utf-8")

            with (
                mock.patch.dict(os.environ, {}, clear=True),
                mock.patch.object(sys, "prefix", temporary_directory),
            ):
                startup()

                with self.assertRaisesRegex(RuntimeError, "RT_CONTEXT_PATH is not set"):
                    get_context()

    def test_startup_rejects_a_missing_profile(self) -> None:
        with mock.patch.dict(
            os.environ, {"RT_CONTEXT_PATH": "/missing/rt-context.json"}, clear=True
        ):
            with self.assertRaisesRegex(RuntimeError, "does not point to an rt run profile"):
                startup()

    def test_external_profile_configures_the_process(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            project_root = root / "project"
            project_root.mkdir()
            profile_path = root / "profiles" / "example" / "0" / "rt-context.json"
            profile_path.parent.mkdir(parents=True)
            profile = {
                "rt_version": "test",
                "project_root": str(project_root),
                "no_editable": False,
                "environment": {"SUITESPEC_VALUE": "run-0"},
                "services": ["testagent"],
                "command": "pytest tests/example.py",
            }
            profile_path.write_text(json.dumps(profile), encoding="utf-8")

            with mock.patch.dict(
                os.environ, {"RT_CONTEXT_PATH": str(profile_path)}, clear=True
            ):
                startup()

                self.assertEqual(get_context(), profile)
                self.assertEqual(os.environ["SUITESPEC_VALUE"], "run-0")
                self.assertEqual(os.environ["PYTHONPATH"], str(project_root))
                self.assertEqual(
                    os.environ["DD_TRACE_AGENT_URL"], "http://localhost:9126"
                )
                self.assertEqual(
                    os.environ["PYTEST_PLUGINS"], "_rt_runtime.pytest_plugin"
                )

    def test_process_cannot_switch_profiles(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            first_path = root / "first.json"
            second_path = root / "second.json"
            base = {
                "rt_version": "test",
                "project_root": str(root),
                "no_editable": False,
                "environment": {},
                "services": [],
                "command": None,
            }
            first_path.write_text(json.dumps({**base, "run": 1}), encoding="utf-8")
            second_path.write_text(json.dumps({**base, "run": 2}), encoding="utf-8")

            with mock.patch.dict(
                os.environ, {"RT_CONTEXT_PATH": str(first_path)}, clear=True
            ):
                startup()
                os.environ["RT_CONTEXT_PATH"] = str(second_path)

                with self.assertRaisesRegex(RuntimeError, "cannot switch run profiles"):
                    get_context()


if __name__ == "__main__":
    unittest.main()
