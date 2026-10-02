import multiprocessing
import queue
import threading

from fastapi import FastAPI

app = FastAPI()
barrier = threading.Barrier(2)
simple = queue.SimpleQueue()
pool = multiprocessing.Pool()


def square(x: int) -> int:
    return x * x


@app.get("/sync")
async def sync_route():
    barrier.wait()  # expect: *
    simple.get()  # expect: *
    pool.starmap(pow, [(1, 2)])  # expect: *
    pool.imap(square, range(3))
    mq = multiprocessing.Queue()
    mq.get()  # expect: *
    simple.put(1)
    return {}
