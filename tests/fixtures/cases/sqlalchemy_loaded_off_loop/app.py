"""A lazy relationship already loaded off the loop (in an offloaded helper) is not loaded again
when the endpoint reads it."""

from fastapi import FastAPI
from fastapi.concurrency import run_in_threadpool
from sqlalchemy import ForeignKey, String
from sqlalchemy.orm import DeclarativeBase, Mapped, Session, mapped_column, relationship


class Base(DeclarativeBase):
    pass


class Team(Base):
    __tablename__ = "teams"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(50))
    players: Mapped[list["Player"]] = relationship()
    coaches: Mapped[list["Coach"]] = relationship()


class Player(Base):
    __tablename__ = "players"
    id: Mapped[int] = mapped_column(primary_key=True)
    team_id: Mapped[int] = mapped_column(ForeignKey("teams.id"))


class Coach(Base):
    __tablename__ = "coaches"
    id: Mapped[int] = mapped_column(primary_key=True)
    team_id: Mapped[int] = mapped_column(ForeignKey("teams.id"))


session = Session()
app = FastAPI()


def load_touching(team_id: int) -> Team | None:
    team = session.get(Team, team_id)
    if team is not None:
        _ = len(team.players)
    return team


def load_refreshing(team_id: int) -> Team | None:
    team = session.get(Team, team_id)
    if team is not None:
        session.refresh(team, attribute_names=["players"])
    return team


def load_refreshing_columns(team_id: int) -> Team | None:
    team = session.get(Team, team_id)
    if team is not None:
        # Without `attribute_names`, lazy relationships stay unloaded.
        session.refresh(team)
    return team


def load_then_commit(team_id: int) -> Team | None:
    team = session.get(Team, team_id)
    if team is not None:
        _ = list(team.players)
        session.commit()  # expires the instance
    return team


def load_plain(team_id: int) -> Team | None:
    return session.get(Team, team_id)


def warm(team: Team) -> None:
    for _ in team.players:
        pass


@app.get("/touched/{team_id}")
async def touched(team_id: int):
    team = await run_in_threadpool(load_touching, team_id)
    if team is None:
        return {}
    players = [p.id for p in team.players]
    coaches = [c.id for c in team.coaches]  # expect: sqlalchemy.orm.session.Session.execute
    return {"players": players, "coaches": coaches}


@app.get("/refreshed/{team_id}")
async def refreshed(team_id: int):
    team = await run_in_threadpool(load_refreshing, team_id)
    if team is None:
        return {}
    return {"players": len(team.players)}


@app.get("/columns/{team_id}")
async def columns(team_id: int):
    team = await run_in_threadpool(load_refreshing_columns, team_id)
    if team is None:
        return {}
    return {"players": len(team.players)}  # expect: sqlalchemy.orm.session.Session.execute


@app.get("/committed/{team_id}")
async def committed(team_id: int):
    team = await run_in_threadpool(load_then_commit, team_id)
    if team is None:
        return {}
    return {"players": len(team.players)}  # expect: sqlalchemy.orm.session.Session.execute


@app.get("/warmed/{team_id}")
async def warmed(team_id: int):
    team = await run_in_threadpool(load_plain, team_id)
    if team is None:
        return {}
    before = len(team.coaches)  # expect: sqlalchemy.orm.session.Session.execute
    await run_in_threadpool(warm, team)
    return {"players": len(team.players), "coaches": before}
