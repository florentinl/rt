from typing import Callable, Iterable, Optional, TypeVar

T = TypeVar("T")

def choice(choices: Iterable[T]) -> Callable[[Optional[T]], None]: ...
def range(min_value: T, max_value: T) -> Callable[[Optional[T]], None]: ...

