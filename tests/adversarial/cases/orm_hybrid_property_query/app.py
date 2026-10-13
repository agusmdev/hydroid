from typing import Iterator

from fastapi import Depends, FastAPI
from fastapi.concurrency import run_in_threadpool
from sqlalchemy import ForeignKey, String, create_engine, func, select
from sqlalchemy.ext.hybrid import hybrid_property
from sqlalchemy.orm import (
    DeclarativeBase,
    Mapped,
    Session,
    mapped_column,
    object_session,
    sessionmaker,
)
from sqlalchemy.pool import StaticPool

engine = create_engine(
    "sqlite://", connect_args={"check_same_thread": False}, poolclass=StaticPool
)
SessionLocal = sessionmaker(bind=engine)


class Base(DeclarativeBase):
    pass


class Order(Base):
    __tablename__ = "orders"
    id: Mapped[int] = mapped_column(primary_key=True)
    customer_id: Mapped[int] = mapped_column(ForeignKey("customers.id"))
    total: Mapped[int] = mapped_column(default=0)


class Customer(Base):
    __tablename__ = "customers"
    id: Mapped[int] = mapped_column(primary_key=True)
    first_name: Mapped[str] = mapped_column(String(50))
    last_name: Mapped[str] = mapped_column(String(50))

    @hybrid_property
    def full_name(self) -> str:
        return f"{self.first_name} {self.last_name}"

    @hybrid_property
    def lifetime_value(self) -> int:
        session = object_session(self)
        assert session is not None
        return session.scalar(
            select(func.coalesce(func.sum(Order.total), 0)).where(Order.customer_id == self.id)
        )

    @lifetime_value.inplace.expression
    @classmethod
    def _lifetime_value_expression(cls):
        return (
            select(func.coalesce(func.sum(Order.total), 0))
            .where(Order.customer_id == cls.id)
            .scalar_subquery()
        )


Base.metadata.create_all(engine)
with SessionLocal.begin() as _seed:
    _seed.add(Customer(id=1, first_name="Ann", last_name="Lee"))
    _seed.add(Order(customer_id=1, total=40))


def get_db() -> Iterator[Session]:
    with SessionLocal() as db:
        yield db


app = FastAPI()


@app.get("/customers/{customer_id}")
async def customer_summary(customer_id: int, db: Session = Depends(get_db)):
    customer = await run_in_threadpool(db.get, Customer, customer_id)
    name = customer.full_name
    ltv = customer.lifetime_value  # expect: sqlalchemy.orm.session.Session.scalar
    return {"name": name, "ltv": ltv}


@app.get("/customers/{customer_id}/ltv-sql")
async def customer_ltv_sql(customer_id: int, db: Session = Depends(get_db)):
    stmt = select(Customer.lifetime_value).where(Customer.id == customer_id)
    ltv = await run_in_threadpool(db.scalar, stmt)
    return {"ltv": ltv}


@app.get("/customers/{customer_id}/sync")
def customer_summary_sync(customer_id: int, db: Session = Depends(get_db)):
    customer = db.get(Customer, customer_id)
    return {"ltv": customer.lifetime_value if customer else 0}
