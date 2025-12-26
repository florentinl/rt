"""
Pytest plugin to strip the global Python-version suffix added by tests/conftest.py.

When loaded with ``-p rt`` this restores the original item names and nodeids so IDE
integration can map collected tests correctly.
"""

import sys
from typing import Iterable

try:
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

except:  # noqa: E722
    pass
