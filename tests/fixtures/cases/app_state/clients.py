import asyncio
import sqlite3
import time


class Ledger:
    def __init__(self, path: str):
        self.conn = sqlite3.connect(path, check_same_thread=False)

    def total(self):
        return self.conn.execute("SELECT 1").fetchone()


class BlockingCache:
    def get(self, key: str):
        time.sleep(0.01)
        return key


class AsyncCache:
    async def get(self, key: str):
        await asyncio.sleep(0)
        return key


class Reports:
    def export(self):
        time.sleep(1)
