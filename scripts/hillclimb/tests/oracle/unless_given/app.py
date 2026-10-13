# `make_msgid()` names the host with `socket.getfqdn()`; given `domain=` it does not (catalog:
# `unless_given = ["domain"]`).
from email.utils import make_msgid

from fastapi import FastAPI

app = FastAPI()


@app.get("/fqdn")
async def with_fqdn():
    return {"id": make_msgid()}  # expect: email.utils.make_msgid


@app.get("/domain")
async def with_domain():
    return {"id": make_msgid(domain="mail.example.com")}


@app.get("/positional")
async def with_positional_domain():
    return {"id": make_msgid("invite", "mail.example.com")}
