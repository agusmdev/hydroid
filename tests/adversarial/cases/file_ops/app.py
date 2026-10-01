import glob
import gzip
import os
import shutil
import tarfile
import tempfile
import zipfile
from pathlib import Path

from fastapi import FastAPI

app = FastAPI()


@app.post("/files")
async def files(src: str, dst: str):
    shutil.move(src, dst)  # expect: *
    os.path.getmtime(dst)  # expect: *
    os.chmod(dst, 0o644)  # expect: *
    tmp = tempfile.NamedTemporaryFile()  # expect: *
    tempfile.mkdtemp()  # expect: *
    tarfile.open("a.tar")  # expect: *
    zipfile.ZipFile("a.zip")  # expect: *
    gzip.open("a.gz")  # expect: *
    glob.glob("*.py")  # expect: *
    os.path.islink(dst)  # expect: *
    os.lstat(dst)  # expect: *
    os.utime(dst)  # expect: *
    os.symlink(src, dst)  # expect: *
    Path(dst).chmod(0o600)  # expect: *
    Path(dst).resolve(strict=True)  # expect: *
    with tempfile.TemporaryDirectory() as scratch:  # expect: tempfile.TemporaryDirectory.__init__ tempfile.TemporaryDirectory.__exit__
        print(scratch)
    os.path.join(src, dst)
    Path(src).name
    return tmp.name
