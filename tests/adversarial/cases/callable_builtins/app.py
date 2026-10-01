import functools
import os

from fastapi import FastAPI

app = FastAPI()


def load(name: str) -> str:
    with open(name) as f:
        return f.read()


def mtime(path: str) -> float:
    return os.stat(path).st_mtime


@app.get("/builtins")
async def builtins_route(paths: list[str]):
    newest = sorted(paths, key=mtime)  # expect: os.stat
    biggest = max(paths, key=os.path.getsize)  # expect: genericpath.getsize
    smallest = min(paths, key=lambda p: os.stat(p).st_size)  # expect: os.stat
    contents = list(map(load, paths))  # expect: builtins.open
    present = list(filter(os.path.exists, paths))  # expect: genericpath.exists
    total = functools.reduce(lambda acc, p: acc + len(load(p)), paths, 0)  # expect: builtins.open
    paths.sort(key=mtime)  # expect: os.stat
    by_len = sorted(paths, key=len)
    return newest, biggest, smallest, contents, present, total, by_len
