import asyncio
import os
import shutil
import tempfile
import zipfile

from fastapi import FastAPI, HTTPException

app = FastAPI()

MEDIA_ROOT = os.path.join(tempfile.gettempdir(), "file_archive_helpers_media")
BACKUP_ROOT = os.path.join(tempfile.gettempdir(), "file_archive_helpers_backups")
os.makedirs(MEDIA_ROOT, exist_ok=True)
os.makedirs(BACKUP_ROOT, exist_ok=True)
with open(os.path.join(MEDIA_ROOT, "cover.txt"), "w") as fh:
    fh.write("cover")


@app.post("/backups")
async def create_backup():
    base = os.path.join(BACKUP_ROOT, "media")
    archive = shutil.make_archive(base, "zip", MEDIA_ROOT)  # expect: *
    return {"archive": os.path.basename(archive)}


@app.post("/backups/offloaded")
async def create_backup_offloaded():
    base = os.path.join(BACKUP_ROOT, "media-bg")
    archive = await asyncio.to_thread(shutil.make_archive, base, "zip", MEDIA_ROOT)
    return {"archive": os.path.basename(archive)}


@app.post("/backups/verify")
async def verify_backup(name: str):
    path = os.path.join(BACKUP_ROOT, os.path.basename(name) + ".zip")
    if not zipfile.is_zipfile(path):  # expect: *
        raise HTTPException(status_code=400, detail="not a zip archive")
    return {"ok": True}


@app.post("/backups/sync-verify")
def verify_backup_sync(name: str):
    path = os.path.join(BACKUP_ROOT, os.path.basename(name) + ".zip")
    return {"ok": zipfile.is_zipfile(path)}
