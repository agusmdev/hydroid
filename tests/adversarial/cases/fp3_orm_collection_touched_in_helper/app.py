from typing import Iterator

from fastapi import Depends, FastAPI
from fastapi.concurrency import run_in_threadpool
from sqlalchemy import ForeignKey, String, create_engine
from sqlalchemy.orm import DeclarativeBase, Mapped, Session, mapped_column, relationship, sessionmaker
from sqlalchemy.pool import StaticPool

engine = create_engine(
    "sqlite://", connect_args={"check_same_thread": False}, poolclass=StaticPool
)
SessionLocal = sessionmaker(bind=engine, expire_on_commit=False)


class Base(DeclarativeBase):
    pass


class Project(Base):
    __tablename__ = "projects"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(50))
    members: Mapped[list["Member"]] = relationship()
    labels: Mapped[list["Label"]] = relationship()


class Member(Base):
    __tablename__ = "members"
    id: Mapped[int] = mapped_column(primary_key=True)
    email: Mapped[str] = mapped_column(String(100))
    project_id: Mapped[int] = mapped_column(ForeignKey("projects.id"))


class Label(Base):
    __tablename__ = "labels"
    id: Mapped[int] = mapped_column(primary_key=True)
    text: Mapped[str] = mapped_column(String(30))
    project_id: Mapped[int] = mapped_column(ForeignKey("projects.id"))


Base.metadata.create_all(engine)
with SessionLocal.begin() as _seed:
    _seed.add(Project(id=1, name="hydra", members=[Member(email="a@x.io")], labels=[Label(text="bug")]))


def get_db() -> Iterator[Session]:
    with SessionLocal() as db:
        yield db


def load_project(db: Session, project_id: int) -> Project | None:
    project = db.get(Project, project_id)
    if project is not None:
        # Touch the collection while still in the worker thread so it is loaded.
        _ = len(project.members)
    return project


app = FastAPI()


@app.get("/projects/{project_id}/members")
async def project_members(project_id: int, db: Session = Depends(get_db)):
    project = await run_in_threadpool(load_project, db, project_id)
    if project is None:
        return {"members": []}
    emails = [m.email for m in project.members]
    return {"project": project.name, "members": emails}


@app.get("/projects/{project_id}/labels")
async def project_labels(project_id: int, db: Session = Depends(get_db)):
    project = await run_in_threadpool(load_project, db, project_id)
    if project is None:
        return {"labels": []}
    texts = [lb.text for lb in project.labels]  # expect: sqlalchemy.orm.session.Session.execute
    return {"project": project.name, "labels": texts}
