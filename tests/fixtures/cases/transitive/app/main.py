from fastapi import APIRouter

from . import services
from .recursion import even, fine_recursion
from .services import load as load_alias

router = APIRouter()


@router.get("/users/{user_id}")
async def get_user(user_id: int):
    user = services.load(user_id)  # expect: requests.api.get
    return user


@router.get("/alias")
async def via_alias():
    return load_alias(1)  # expect: requests.api.get


@router.get("/recursion")
async def recursive():
    fine_recursion(10)
    return even(10)  # expect: time.sleep


@router.get("/coroutine")
async def coroutine_factory():
    # Calling a sync function that only builds a coroutine: the coroutine is awaited here,
    # and `services.fetch_async` is analyzed on its own.
    return await services.make_coroutine()
