import asyncio
from contextlib import asynccontextmanager

import requests
from fastapi import FastAPI, Request


class BillingClient:
    def __init__(self, base_url: str):
        self.base_url = base_url
        self.session = requests.Session()

    def invoice(self, invoice_id: str):
        return self.session.get(f"{self.base_url}/invoices/{invoice_id}")


@asynccontextmanager
async def lifespan(app: FastAPI):
    yield {"billing": BillingClient("http://billing.internal")}


app = FastAPI(lifespan=lifespan)


@app.get("/invoices/{invoice_id}")
async def get_invoice(invoice_id: str, request: Request):
    billing = request.state.billing
    resp = billing.invoice(invoice_id)  # expect: requests.sessions.Session.get
    return {"ok": bool(resp)}


@app.get("/invoices/{invoice_id}/offloaded")
async def get_invoice_offloaded(invoice_id: str, request: Request):
    resp = await asyncio.to_thread(request.state.billing.invoice, invoice_id)
    return {"ok": bool(resp)}


@app.get("/invoices/{invoice_id}/sync")
def get_invoice_sync(invoice_id: str, request: Request):
    resp = request.state.billing.invoice(invoice_id)
    return {"ok": bool(resp)}
