import select
import smtplib
import socket
import ssl

from fastapi import FastAPI

app = FastAPI()


@app.post("/mail")
async def mail(host: str):
    server = smtplib.SMTP_SSL(host)  # expect: *
    server.send_message(None)  # expect: smtplib.SMTP.send_message
    sock = socket.socket()
    select.select([sock], [], [], 1.0)  # expect: *
    sock.sendfile(open("x", "rb"))  # expect: * builtins.open
    ctx = ssl.create_default_context()
    wrapped = ctx.wrap_socket(sock, server_hostname=host)  # expect: *
    wrapped.do_handshake()  # expect: *
    socket.getfqdn()  # expect: *
    return {}
