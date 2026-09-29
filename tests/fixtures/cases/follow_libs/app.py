import slowlib
from fastapi import FastAPI

app = FastAPI()


@app.get("/report/{report_id}")
async def report(report_id: int):
    total = slowlib.cached_total()
    await slowlib.refresh_properly()
    await slowlib.refresh()  # expect: time.sleep via route GET /report/{report_id}
    return slowlib.fetch_report(report_id)  # expect: requests.api.get via route GET /report/{report_id}
