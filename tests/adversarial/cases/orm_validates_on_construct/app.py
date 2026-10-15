import redis
from fastapi import FastAPI
from fastapi.concurrency import run_in_threadpool
from pydantic import BaseModel
from sqlalchemy import String
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, validates

blocklist = redis.Redis(host="localhost", port=6379)


class Base(DeclarativeBase):
    pass


class Customer(Base):
    __tablename__ = "customers"
    id: Mapped[int] = mapped_column(primary_key=True)
    email: Mapped[str] = mapped_column(String(200))
    nickname: Mapped[str] = mapped_column(String(50), default="")

    @validates("email")
    def _check_email(self, key: str, value: str) -> str:
        domain = value.rpartition("@")[2].lower()
        if blocklist.sismember("blocked_domains", domain):
            raise ValueError("email domain is blocked")
        return value


class CustomerIn(BaseModel):
    email: str
    nickname: str = ""


app = FastAPI()


def build_customer(payload: CustomerIn) -> Customer:
    return Customer(email=payload.email, nickname=payload.nickname)


@app.post("/customers")
async def create_customer(payload: CustomerIn):
    customer = Customer(email=payload.email, nickname=payload.nickname)  # expect: redis.commands.core.SetCommands.sismember
    return {"email": customer.email}


@app.patch("/customers/me")
async def rename_customer(payload: CustomerIn):
    customer = Customer(nickname="x")
    customer.nickname = payload.nickname
    customer.email = payload.email  # expect: redis.commands.core.SetCommands.sismember
    return {"email": customer.email, "nickname": customer.nickname}


@app.post("/customers/offloaded")
async def create_customer_offloaded(payload: CustomerIn):
    try:
        customer = await run_in_threadpool(build_customer, payload)
    except Exception:
        return {"email": None}
    return {"email": customer.email}
