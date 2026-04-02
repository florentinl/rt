from collections.abc import Sequence

def cli_main(args: list[str]) -> int: ...

class Venv:
    def __init__(
        self,
        name: str | None = None,
        command: str | None = None,
        pys: str | Sequence[str] | None = None,
        pkgs: dict[str, str | list[str]] | None = None,
        env: dict[str, str | list[str]] | None = None,
        venvs: list[Venv] | None = None,
        create: bool | None = None,
        skip_dev_install: bool | None = None,
    ) -> None: ...
