import asyncio

import pymysql
import pymysql.cursors
from fastapi import FastAPI

app = FastAPI()

db = pymysql.connect(
    host="mysql",
    user="app",
    password="secret",
    database="shop",
    cursorclass=pymysql.cursors.DictCursor,
    defer_connect=True,
)


def count_orders_sync() -> int:
    with db.cursor() as cur:
        cur.execute("SELECT COUNT(*) AS n FROM orders")
        return cur.fetchone()["n"]


@app.get("/orders/{order_id}")
async def get_order(order_id: int):
    with db.cursor() as cur:
        cur.execute("SELECT * FROM orders WHERE id = %s", (order_id,))  # expect: *
        row = cur.fetchone()  # expect: *
    total = await asyncio.to_thread(count_orders_sync)
    return {"order": row, "total": total}


@app.get("/orders")
def list_orders():
    with db.cursor() as cur:
        cur.execute("SELECT * FROM orders")
        return cur.fetchall()
