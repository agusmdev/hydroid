import redis
from fastapi import FastAPI

app = FastAPI()
r = redis.Redis()


@app.get("/lock")
async def locked():
    with r.lock("job"):  # expect: *
        pass
    lock = r.lock("other")
    lock.acquire()  # expect: *
    lock.release()  # expect: *
    return {}


@app.get("/pubsub")
async def pubsub():
    p = r.pubsub()
    p.subscribe("channel")  # expect: *
    message = p.get_message(timeout=1)  # expect: *
    for item in p.listen():  # expect: *
        print(item)
    return message
