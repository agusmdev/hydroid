import requests
from fastapi import FastAPI

app = FastAPI()

session = requests.Session()
session.headers["User-Agent"] = "gateway/1.0"

DISPATCH = {
    "GET": session.get,
    "POST": session.post,
    "DELETE": session.delete,
}


@app.get("/proxy/{path}")
async def proxy(path: str):
    send = DISPATCH["GET"]
    resp = send(f"https://upstream.internal/{path}", timeout=5)  # expect: *
    return {"status": resp.status_code}


@app.post("/proxy/{path}")
async def proxy_post(path: str):
    resp = DISPATCH["POST"](f"https://upstream.internal/{path}", json={})  # expect: *
    return {"status": resp.status_code}


@app.delete("/proxy/{path}")
def proxy_delete(path: str):
    try:
        DISPATCH["DELETE"](f"https://upstream.internal/{path}")
    except requests.RequestException:
        pass
    return {}
