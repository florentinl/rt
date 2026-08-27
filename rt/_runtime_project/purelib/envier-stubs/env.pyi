from typing import Any, Callable, ClassVar, Dict, FrozenSet, Generic, Iterator, List, Literal, Mapping, NamedTuple, Optional, Tuple, Type, TypeVar, Union, overload

T = TypeVar("T")
K = TypeVar("K")
V = TypeVar("V")
_E = TypeVar("_E", bound="Env")

DeprecationInfo = Tuple[str, Optional[str], Optional[str]]
MapType = Union[Callable[[str], Any], Callable[[str, str], Tuple[Any, Any]]]

class NoDefaultType:
    def __str__(self) -> str: ...

NoDefault: NoDefaultType

class HelpInfo(NamedTuple):
    name: str
    type: str
    default: str
    help: str

class EnvVariable(Generic[T]):
    type: object
    name: str
    parser: Optional[Callable[[str], T]]
    validator: Optional[Callable[[T], None]]
    map: Optional[MapType]
    default: Union[T, NoDefaultType]
    deprecations: Optional[List[DeprecationInfo]]
    private: bool
    help: Optional[str]
    help_type: Optional[str]
    help_default: Optional[str]
    _full_name: str
    def __init__(
        self,
        type: object,
        name: str,
        parser: Optional[Callable[[str], T]] = ...,
        validator: Optional[Callable[[T], None]] = ...,
        map: Optional[MapType] = ...,
        default: Union[T, NoDefaultType] = ...,
        deprecations: Optional[List[DeprecationInfo]] = ...,
        private: bool = ...,
        help: Optional[str] = ...,
        help_type: Optional[str] = ...,
        help_default: Optional[str] = ...,
    ) -> None: ...
    @property
    def full_name(self) -> str: ...
    @overload
    def __get__(self, instance: None, owner: Type[Env]) -> EnvVariable[T]: ...
    @overload
    def __get__(self, instance: Env, owner: Type[Env]) -> T: ...
    def __set__(self, instance: Env, value: T) -> None: ...
    def __call__(self, env: Env, prefix: str) -> T: ...

class DerivedVariable(Generic[T]):
    type: object
    derivation: Callable[..., T]
    def __init__(self, type: object, derivation: Callable[..., T]) -> None: ...
    @overload
    def __get__(self, instance: None, owner: Type[Env]) -> DerivedVariable[T]: ...
    @overload
    def __get__(self, instance: Env, owner: Type[Env]) -> T: ...
    def __set__(self, instance: Env, value: T) -> None: ...
    def __call__(self, env: Env) -> T: ...

_Declaration = Union[EnvVariable[Any], DerivedVariable[Any]]

class EnvMeta(type):
    def __new__(
        cls, name: str, bases: Tuple[Type[Any], ...], ns: Dict[str, Any]
    ) -> Any: ...

class Env(metaclass=EnvMeta):
    __truthy__: ClassVar[FrozenSet[str]]
    __prefix__: ClassVar[str]
    __item__: ClassVar[Optional[str]]
    __item_separator__: ClassVar[str]
    __value_separator__: ClassVar[str]
    source: Mapping[str, str]
    parent: Optional[Env]
    dynamic: Dict[str, str]
    _full_prefix: str
    @property
    def spec(self: _E) -> Type[_E]: ...
    @spec.setter
    def spec(self: _E, value: Type[_E]) -> None: ...
    def __init__(
        self,
        source: Optional[Mapping[str, str]] = ...,
        parent: Optional[Env] = ...,
        dynamic: Optional[Dict[str, str]] = ...,
    ) -> None: ...
    @classmethod
    @overload
    def var(
        cls,
        type: Type[T],
        name: str,
        parser: Optional[Callable[[str], T]] = ...,
        validator: Optional[Callable[[T], None]] = ...,
        map: Optional[MapType] = ...,
        default: Union[T, NoDefaultType] = ...,
        deprecations: Optional[List[DeprecationInfo]] = ...,
        private: bool = ...,
        help: Optional[str] = ...,
        help_type: Optional[str] = ...,
        help_default: Optional[str] = ...,
    ) -> EnvVariable[T]: ...
    @classmethod
    @overload
    def var(
        cls,
        type: object,
        name: str,
        parser: Optional[Callable[[str], Any]] = ...,
        validator: Optional[Callable[[Any], None]] = ...,
        map: Optional[MapType] = ...,
        default: Any = ...,
        deprecations: Optional[List[DeprecationInfo]] = ...,
        private: bool = ...,
        help: Optional[str] = ...,
        help_type: Optional[str] = ...,
        help_default: Optional[str] = ...,
    ) -> EnvVariable[Any]: ...
    @classmethod
    @overload
    def v(
        cls,
        type: Type[T],
        name: str,
        parser: Optional[Callable[[str], T]] = ...,
        validator: Optional[Callable[[T], None]] = ...,
        map: Optional[MapType] = ...,
        default: Union[T, NoDefaultType] = ...,
        deprecations: Optional[List[DeprecationInfo]] = ...,
        private: bool = ...,
        help: Optional[str] = ...,
        help_type: Optional[str] = ...,
        help_default: Optional[str] = ...,
    ) -> EnvVariable[T]: ...
    @classmethod
    @overload
    def v(
        cls,
        type: object,
        name: str,
        parser: Optional[Callable[[str], Any]] = ...,
        validator: Optional[Callable[[Any], None]] = ...,
        map: Optional[MapType] = ...,
        default: Any = ...,
        deprecations: Optional[List[DeprecationInfo]] = ...,
        private: bool = ...,
        help: Optional[str] = ...,
        help_type: Optional[str] = ...,
        help_default: Optional[str] = ...,
    ) -> EnvVariable[Any]: ...
    @classmethod
    @overload
    def der(cls, type: Type[T], derivation: Callable[..., T]) -> DerivedVariable[T]: ...
    @classmethod
    @overload
    def der(cls, type: object, derivation: Callable[..., T]) -> DerivedVariable[T]: ...
    @classmethod
    @overload
    def d(cls, type: Type[T], derivation: Callable[..., T]) -> DerivedVariable[T]: ...
    @classmethod
    @overload
    def d(cls, type: object, derivation: Callable[..., T]) -> DerivedVariable[T]: ...
    @classmethod
    @overload
    def items(
        cls, recursive: bool = ..., include_derived: Literal[False] = ...
    ) -> Iterator[Tuple[str, EnvVariable[Any]]]: ...
    @classmethod
    @overload
    def items(
        cls, *, include_derived: bool
    ) -> Iterator[Tuple[str, _Declaration]]: ...
    @classmethod
    @overload
    def items(
        cls, recursive: bool, include_derived: bool
    ) -> Iterator[Tuple[str, _Declaration]]: ...
    @classmethod
    def keys(
        cls, recursive: bool = ..., include_derived: bool = ...
    ) -> Iterator[str]: ...
    @classmethod
    @overload
    def values(
        cls, recursive: bool = ..., include_derived: Literal[False] = ...
    ) -> Iterator[EnvVariable[Any]]: ...
    @classmethod
    @overload
    def values(
        cls, *, include_derived: bool
    ) -> Iterator[_Declaration]: ...
    @classmethod
    @overload
    def values(
        cls, recursive: bool, include_derived: bool
    ) -> Iterator[_Declaration]: ...
    @classmethod
    def include(
        cls,
        env_spec: Type[Env],
        namespace: Optional[str] = ...,
        overwrite: bool = ...,
    ) -> None: ...
    @classmethod
    def help_info(
        cls, recursive: bool = ..., include_private: bool = ...
    ) -> List[HelpInfo]: ...
