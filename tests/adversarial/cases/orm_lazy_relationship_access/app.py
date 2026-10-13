from typing import Iterator

from fastapi import Depends, FastAPI
from fastapi.concurrency import run_in_threadpool
from sqlalchemy import ForeignKey, String, create_engine
from sqlalchemy.orm import (
    DeclarativeBase,
    Mapped,
    Session,
    mapped_column,
    relationship,
    selectinload,
    sessionmaker,
)
from sqlalchemy.pool import StaticPool

engine = create_engine(
    "sqlite://", connect_args={"check_same_thread": False}, poolclass=StaticPool
)
SessionLocal = sessionmaker(bind=engine, expire_on_commit=False)


class Base(DeclarativeBase):
    pass


class Author(Base):
    __tablename__ = "authors"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(50))
    books: Mapped[list["Book"]] = relationship(back_populates="author")


class Book(Base):
    __tablename__ = "books"
    id: Mapped[int] = mapped_column(primary_key=True)
    title: Mapped[str] = mapped_column(String(100))
    author_id: Mapped[int] = mapped_column(ForeignKey("authors.id"))
    author: Mapped[Author] = relationship(back_populates="books")


Base.metadata.create_all(engine)
with SessionLocal.begin() as _seed:
    _seed.add(Author(id=1, name="Ann", books=[Book(title="A"), Book(title="B")]))


def get_db() -> Iterator[Session]:
    with SessionLocal() as db:
        yield db


app = FastAPI()


@app.get("/authors/{author_id}/books")
async def author_books(author_id: int, db: Session = Depends(get_db)):
    author = await run_in_threadpool(db.get, Author, author_id)
    titles = [b.title for b in author.books]  # expect: sqlalchemy.orm.session.Session.execute
    return {"author": author.name, "titles": titles}


@app.get("/authors/{author_id}/books-eager")
async def author_books_eager(author_id: int, db: Session = Depends(get_db)):
    author = await run_in_threadpool(
        db.get, Author, author_id, options=[selectinload(Author.books)]
    )
    titles = [b.title for b in author.books]
    return {"author": author.name, "titles": titles}


@app.get("/books/{book_id}/author")
def book_author(book_id: int, db: Session = Depends(get_db)):
    book = db.get(Book, book_id)
    return {"author": book.author.name if book else None}
