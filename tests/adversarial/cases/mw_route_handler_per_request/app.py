import asyncio
import json
from pathlib import Path
from typing import Any, Callable, Coroutine

from fastapi import APIRouter, Depends, FastAPI, Request, Response
from fastapi.routing import APIRoute

AUDIT_POLICY = Path(__file__).with_name("audit_policy.json")


def load_policy() -> dict[str, str]:
    return json.loads(AUDIT_POLICY.read_text())


class AuditedRoute(APIRoute):
    """Attaches the route's audit level (from a JSON policy file) before building its handler."""

    audit_level: str = "basic"

    def get_route_handler(self) -> Callable[[Request], Coroutine[Any, Any, Response]]:
        # Looks like one-time setup at import, but FastAPI also calls get_route_handler() in
        # APIRoute.handle() on every request to a route reached through include_router.
        policy = json.loads(AUDIT_POLICY.read_text())  # expect: pathlib.Path.read_text
        self.audit_level = policy.get(self.name, "basic")
        return super().get_route_handler()


async def audit_level(request: Request) -> str:
    route = request.scope.get("route")
    return getattr(route, "audit_level", "basic")


router = APIRouter(route_class=AuditedRoute)


@router.get("/invoices")
async def list_invoices(level: str = Depends(audit_level)):
    return {"invoices": [], "audit": level}


@router.get("/policy")
async def show_policy():
    # Offloaded: fine.
    return await asyncio.to_thread(load_policy)


app = FastAPI()
app.include_router(router, prefix="/billing")
