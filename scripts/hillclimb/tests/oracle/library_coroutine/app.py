# An async library function that blocks inside (fake_sdk.fetch calls requests.get): the loop is
# blocked, and the oracle records it at the case's `await` line.
import fake_sdk
from fastapi import FastAPI

app = FastAPI()


@app.get("/")
async def index():
    body = await fake_sdk.fetch("http://upstream/")  # expect: requests.api.get
    return {"body": body}
