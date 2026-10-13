import httpx
import requests
from fastapi import Depends, FastAPI

app = FastAPI()


def get_billing_client():
    with httpx.Client(base_url="https://billing.internal", timeout=5) as client:
        yield client


def get_session():
    session = requests.Session()
    session.headers["Authorization"] = "Bearer token"
    return session


@app.get("/invoices")
async def invoices(client=Depends(get_billing_client)):
    resp = client.get("/invoices")  # expect: *
    return {"status": resp.status_code}


@app.get("/users/{user_id}")
async def user(user_id: str, session=Depends(get_session)):
    resp = session.get(f"https://users.internal/{user_id}")  # expect: *
    return {"status": resp.status_code}


@app.get("/invoices-sync")
def invoices_sync(client=Depends(get_billing_client)):
    try:
        client.get("/invoices")
    except httpx.HTTPError:
        pass
    return {}
