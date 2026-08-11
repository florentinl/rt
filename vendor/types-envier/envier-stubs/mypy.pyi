from typing import Type

from mypy.plugin import Plugin

class EnvierPlugin(Plugin): ...

def plugin(version: str) -> Type[EnvierPlugin]: ...
