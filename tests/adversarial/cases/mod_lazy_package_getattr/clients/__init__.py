"""Client modules are imported lazily (PEP 562) so the app starts fast."""
import importlib
from types import ModuleType

_LAZY = {"geo", "pricing"}


def __getattr__(name: str) -> ModuleType:
    if name in _LAZY:
        module = importlib.import_module(f".{name}", __name__)
        globals()[name] = module
        return module
    raise AttributeError(name)
