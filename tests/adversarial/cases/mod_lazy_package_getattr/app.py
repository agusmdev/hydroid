from fastapi import FastAPI

import clients

app = FastAPI()


@app.get("/resolve/{host}")
async def resolve(host: str):
    return {"ip": clients.geo.resolve(host)}  # expect: _socket.gethostbyname


@app.get("/quote/{sku}")
async def quote(sku: str):
    return {"price": clients.pricing.quote(sku)}


@app.get("/resolve-sync/{host}")
def resolve_sync(host: str):
    return {"ip": clients.geo.resolve(host)}
