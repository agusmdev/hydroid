import asyncio
from typing import Annotated

from fastapi import Depends, FastAPI
from sqlalchemy import func
from sqlalchemy.pool import StaticPool
from sqlmodel import Field, Session, SQLModel, create_engine, select


class Hero(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    name: str
    age: int | None = None


engine = create_engine(
    "sqlite://", connect_args={"check_same_thread": False}, poolclass=StaticPool
)
SQLModel.metadata.create_all(engine)
app = FastAPI()


def get_session():
    with Session(engine) as session:
        yield session


SessionDep = Annotated[Session, Depends(get_session)]


def age_stats(session: Session):
    return session.execute(select(func.min(Hero.age), func.max(Hero.age))).one()


@app.get("/heroes/stats")
async def hero_stats(session: SessionDep):
    row = session.execute(select(func.count(Hero.id), func.avg(Hero.age))).one()  # expect: *
    extremes = await asyncio.to_thread(age_stats, session)
    return {"count": row[0], "avg": row[1], "extremes": str(extremes)}


@app.get("/heroes/names")
def hero_names(session: SessionDep):
    return session.execute(select(Hero.name)).scalars().all()
