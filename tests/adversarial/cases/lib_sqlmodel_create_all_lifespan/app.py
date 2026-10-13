import asyncio
from contextlib import asynccontextmanager

from fastapi import FastAPI
from sqlalchemy.pool import StaticPool
from sqlmodel import Field, Session, SQLModel, create_engine, select


class Hero(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    name: str


engine = create_engine(
    "sqlite://", connect_args={"check_same_thread": False}, poolclass=StaticPool
)


def create_db_and_tables() -> None:
    SQLModel.metadata.create_all(engine)


@asynccontextmanager
async def lifespan(app: FastAPI):
    SQLModel.metadata.create_all(engine)  # expect: *
    await asyncio.to_thread(create_db_and_tables)
    yield
    await asyncio.to_thread(SQLModel.metadata.drop_all, engine)


app = FastAPI(lifespan=lifespan)


@app.get("/heroes")
def list_heroes():
    with Session(engine) as session:
        return session.exec(select(Hero)).all()
