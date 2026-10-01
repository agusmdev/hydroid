from fastapi import FastAPI
from sqlalchemy import Integer, String, create_engine, text
from sqlalchemy.orm import DeclarativeBase, Mapped, Session, mapped_column

app = FastAPI()
engine = create_engine("sqlite://")


class Base(DeclarativeBase):
    pass


class User(Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    name: Mapped[str] = mapped_column(String)


@app.get("/users")
async def users():
    session = Session(engine)
    for user in session.query(User):  # expect: *
        print(user)
    names = [u.name for u in session.query(User).filter(User.id > 1)]  # expect: *
    session.query(User).filter(User.id == 1).delete()  # expect: *
    session.query(User).update({"name": "x"})  # expect: *
    session.bulk_save_objects([User(name="a")])  # expect: *
    with engine.connect() as conn:  # expect: sqlalchemy.engine.base.Engine.connect
        conn.exec_driver_sql("select 1")  # expect: *
    return names
