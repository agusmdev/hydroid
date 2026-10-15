import asyncio
from dataclasses import dataclass, field

import requests
from fastapi import Depends, FastAPI
from pydantic import BaseModel


# --- domain events ---------------------------------------------------------


@dataclass
class UserRegistered:
    email: str


@dataclass
class UserDeactivated:
    email: str


def send_welcome_email(event: UserRegistered) -> None:
    requests.post(
        "https://mailer.internal/v1/send",
        json={"to": event.email, "template": "welcome"},
        timeout=2,
    )


def record_signup_metric(event: UserRegistered) -> None:
    print("signup", event.email)


def log_deactivation(event: UserDeactivated) -> None:
    print("deactivated", event.email)


EVENT_HANDLERS = {
    UserRegistered: [record_signup_metric, send_welcome_email],
    UserDeactivated: [log_deactivation],
}


# --- unit of work ----------------------------------------------------------


@dataclass
class UnitOfWork:
    users: dict[str, dict] = field(default_factory=dict)
    events: list = field(default_factory=list)

    async def __aenter__(self) -> "UnitOfWork":
        return self

    async def __aexit__(self, *exc) -> None:
        if exc[0] is None:
            await self.commit()

    async def commit(self) -> None:
        pending, self.events = self.events, []
        for event in pending:
            for handler in EVENT_HANDLERS[type(event)]:
                handler(event)  # expect: requests.api.post


def get_uow() -> UnitOfWork:
    return UnitOfWork()


# --- service ---------------------------------------------------------------


class UserService:
    def __init__(self, uow: UnitOfWork) -> None:
        self.uow = uow

    async def register(self, email: str) -> dict:
        async with self.uow:
            user = {"email": email, "active": True}
            self.uow.users[email] = user
            self.uow.events.append(UserRegistered(email=email))
        return user


def get_user_service(uow: UnitOfWork = Depends(get_uow)) -> UserService:
    return UserService(uow)


app = FastAPI()


class RegisterIn(BaseModel):
    email: str


@app.post("/users")
async def register(body: RegisterIn, svc: UserService = Depends(get_user_service)):
    return await svc.register(body.email)


@app.post("/users/offloaded")
async def register_offloaded(body: RegisterIn):
    await asyncio.to_thread(send_welcome_email, UserRegistered(email=body.email))
    return {"email": body.email}
