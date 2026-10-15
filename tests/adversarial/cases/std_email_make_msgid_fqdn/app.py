import asyncio
from email.message import EmailMessage
from email.utils import formatdate, make_msgid

from fastapi import FastAPI
from pydantic import BaseModel

app = FastAPI()

MAIL_DOMAIN = "mail.example.com"


class Invite(BaseModel):
    to: str
    team: str


def build_message(invite: Invite, msgid: str) -> EmailMessage:
    msg = EmailMessage()
    msg["From"] = f"noreply@{MAIL_DOMAIN}"
    msg["To"] = invite.to
    msg["Subject"] = f"Join {invite.team}"
    msg["Date"] = formatdate(localtime=True)
    msg["Message-ID"] = msgid
    msg.set_content(f"You were invited to {invite.team}.")
    return msg


@app.post("/invites/preview")
async def preview_invite(invite: Invite):
    msg = build_message(invite, make_msgid())  # expect: socket.getfqdn
    return {"message_id": msg["Message-ID"], "subject": msg["Subject"]}


@app.post("/invites/preview-fixed")
async def preview_invite_fixed(invite: Invite):
    msg = build_message(invite, make_msgid(domain=MAIL_DOMAIN))
    return {"message_id": msg["Message-ID"]}


@app.post("/invites/preview-threaded")
async def preview_invite_threaded(invite: Invite):
    msgid = await asyncio.to_thread(make_msgid)
    return {"message_id": build_message(invite, msgid)["Message-ID"]}
