"""SQLAlchemy attribute hooks: lazy relationship loads and `@validates` validators."""

import time

from fastapi import Depends, FastAPI
from fastapi.concurrency import run_in_threadpool
from sqlalchemy import ForeignKey, String, select
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import (
    DeclarativeBase,
    Mapped,
    Session,
    joinedload,
    mapped_column,
    relationship,
    selectinload,
    validates,
)


class Base(DeclarativeBase):
    pass


class Author(Base):
    __tablename__ = "authors"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(50))
    books: Mapped[list["Book"]] = relationship(back_populates="author")
    # Explicitly eager (or never loading) strategies emit no query on access.
    awards: Mapped[list["Award"]] = relationship(lazy="selectin")
    reviews: Mapped[list["Review"]] = relationship(lazy="raise")
    fans: Mapped[list["Fan"]] = relationship(lazy="joined")

    @validates("name")
    def _check_name(self, key: str, value: str) -> str:
        time.sleep(0.1)
        return value


class Book(Base):
    __tablename__ = "books"
    id: Mapped[int] = mapped_column(primary_key=True)
    author_id: Mapped[int] = mapped_column(ForeignKey("authors.id"))
    author: Mapped[Author] = relationship(back_populates="books", lazy="select")


class Award(Base):
    __tablename__ = "awards"
    id: Mapped[int] = mapped_column(primary_key=True)
    author_id: Mapped[int] = mapped_column(ForeignKey("authors.id"))


class Review(Base):
    __tablename__ = "reviews"
    id: Mapped[int] = mapped_column(primary_key=True)
    author_id: Mapped[int] = mapped_column(ForeignKey("authors.id"))


class Fan(Base):
    __tablename__ = "fans"
    id: Mapped[int] = mapped_column(primary_key=True)
    author_id: Mapped[int] = mapped_column(ForeignKey("authors.id"))


def get_db() -> Session:
    return Session()


def load_author_with_books(db: Session, author_id: int) -> Author | None:
    return db.scalars(select(Author).options(selectinload(Author.books)).where(Author.id == author_id)).first()


app = FastAPI()


@app.get("/lazy/{author_id}")
async def lazy(author_id: int, db: Session = Depends(get_db)):
    author = await run_in_threadpool(db.get, Author, author_id)
    return [b.id for b in author.books]  # expect: sqlalchemy.orm.session.Session.execute


@app.get("/many-to-one/{book_id}")
async def many_to_one(book_id: int, db: Session = Depends(get_db)):
    book = await run_in_threadpool(db.get, Book, book_id)
    return book.author.id  # expect: sqlalchemy.orm.session.Session.execute


@app.get("/eager-strategies/{author_id}")
async def eager_strategies(author_id: int, db: Session = Depends(get_db)):
    author = await run_in_threadpool(db.get, Author, author_id)
    return [len(author.awards), len(author.reviews), len(author.fans)]


@app.get("/eager-option/{author_id}")
async def eager_option(author_id: int, db: Session = Depends(get_db)):
    author = await run_in_threadpool(db.get, Author, author_id, options=[joinedload(Author.books)])
    return [b.id for b in author.books]


@app.get("/eager-helper/{author_id}")
async def eager_helper(author_id: int, db: Session = Depends(get_db)):
    author = await run_in_threadpool(load_author_with_books, db, author_id)
    return [b.id for b in author.books]


@app.get("/transient")
async def transient():
    author = Author()
    return len(author.books)


@app.get("/async-session/{author_id}")
async def async_session(author_id: int, session: AsyncSession):
    author = await session.get(Author, author_id)
    return [b.id for b in author.books] if author else []


@app.post("/validated")
async def validated(name: str):
    author = Author(name=name)  # expect: time.sleep
    author.name = name.title()  # expect: time.sleep
    author.id = 3
    return Author(id=1).id


async def fetch_book(book_id: int) -> Book:
    return Book(id=book_id)


@app.get("/awaited/{book_id}")
async def awaited(book_id: int):
    # Obtained by awaiting (the async ORM): a lazy load raises instead of blocking.
    book = await fetch_book(book_id)
    first = book
    return [first.author.id, (await fetch_book(book_id)).author.id]


@app.get("/async-orm-param/{book_id}")
async def async_orm_param(book_id: int, session: AsyncSession, db: Session = Depends(get_db)):
    book = await run_in_threadpool(db.get, Book, book_id)
    return book.author.id
