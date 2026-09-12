"""The receiver's type comes from a call in another module.

`s` is only typed by what `make_store()` returns, which no AST walk can see.
Name-based scoring then prefers `Cache.get` — it is the `get` this file
imports a path to — and gets the answer exactly wrong.
"""

from cache import Cache
from factory import make_store


def run():
    s = make_store()
    return s.get("k")


def clear(c: Cache):
    return c.get("k")
