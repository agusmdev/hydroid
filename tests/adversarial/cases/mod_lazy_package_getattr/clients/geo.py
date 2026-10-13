import socket


def resolve(host: str) -> str:
    return socket.gethostbyname(host)
