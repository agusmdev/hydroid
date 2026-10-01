import shutil

from fastapi import FastAPI, UploadFile

app = FastAPI()


@app.post("/upload")
async def upload(file: UploadFile):
    head = file.file.read(100)  # expect: *
    with open("/tmp/out", "wb") as out:  # expect: builtins.open
        shutil.copyfileobj(file.file, out)  # expect: *
    rest = await file.read()
    return len(head) + len(rest)
