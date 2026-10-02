import threading

from fastapi import FastAPI

app = FastAPI()

sem = threading.Semaphore(2)
bounded = threading.BoundedSemaphore(2)
cond = threading.Condition()
rlock = threading.RLock()


@app.get("/primitives")
async def primitives():
    with sem:  # expect: *
        pass
    with bounded:  # expect: *
        pass
    with cond:  # expect: *
        pass
    with rlock:  # expect: *
        pass
    rlock.acquire()  # expect: *
    rlock.release()
    sem.release()
    return {}
