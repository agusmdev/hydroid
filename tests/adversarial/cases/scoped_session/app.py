from fastapi import FastAPI
from sqlalchemy import create_engine, text
from sqlalchemy.orm import scoped_session, sessionmaker

app = FastAPI()
engine = create_engine("sqlite://")
Db = scoped_session(sessionmaker(bind=engine))


@app.get("/scoped")
async def scoped():
    rows = Db.execute(text("select 1"))  # expect: *
    Db.commit()  # expect: *
    value = Db.scalar(text("select 1"))  # expect: *
    return rows, value
