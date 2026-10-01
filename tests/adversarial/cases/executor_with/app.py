import time
from concurrent.futures import ThreadPoolExecutor, as_completed

from fastapi import FastAPI

app = FastAPI()


def work(n: int) -> int:
    time.sleep(n)
    return n


@app.get("/executor")
async def executor_route():
    with ThreadPoolExecutor() as pool:  # expect: *
        futures = [pool.submit(work, n) for n in range(3)]
    for done in as_completed(futures):  # expect: *
        print(done)
    results = list(pool.map(work, range(3)))  # expect: *
    return results
