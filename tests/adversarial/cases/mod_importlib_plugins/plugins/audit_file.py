import json
import os
import tempfile

AUDIT_PATH = os.path.join(tempfile.gettempdir(), "audit.log")


def record(event: str, payload: dict) -> None:
    with open(AUDIT_PATH, "a") as fh:
        fh.write(json.dumps({"event": event, **payload}) + "\n")
