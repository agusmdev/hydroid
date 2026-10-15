import redis
from fastapi import FastAPI
from starlette.convertors import Convertor, register_url_convertor

cache = redis.Redis(host="cache.internal")


class TenantConvertor(Convertor):
    """`/t/{tenant:tenant}/...`: resolves a tenant slug to its tenant id while matching."""

    regex = "[a-z0-9-]+"

    def convert(self, value: str) -> str:
        tenant_id = cache.get(f"tenant:{value}")  # expect: redis.commands.core.BasicKeyCommands.get
        return value if tenant_id is None else str(tenant_id)

    def to_string(self, value: str) -> str:
        return str(value)


register_url_convertor("tenant", TenantConvertor())

app = FastAPI()


@app.get("/t/{tenant:tenant}/dashboard")
async def dashboard(tenant: str) -> dict:
    return {"tenant": tenant}


@app.get("/t/{tenant:tenant}/export")
def export(tenant: str) -> dict:
    return {"tenant": tenant}
