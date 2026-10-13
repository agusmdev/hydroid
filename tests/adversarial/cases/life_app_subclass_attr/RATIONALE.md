# life_app_subclass_attr

A `FastAPI` subclass (`ShopApp`) carries typed attributes and helper methods; the lifespan fills
`app.pricing`. Endpoints reach the app through `request.app`, which Starlette types as `Any`.
With an explicit annotation (`shop: ShopApp = request.app`) hydroid finds the blocking
`Path.read_text` (line 38, control). Without one, `request.app.refresh_pricing()` (line 43) is
missed, although `request.app` is always the application the route is mounted on: `app`, a
`ShopApp` instance, whose `refresh_pricing` -> `PricingTable.reload` -> `pathlib.Path.read_text`.

Runtime: POST /prices/refresh runs `refresh_pricing` on the loop thread; the oracle records
`pathlib.Path.read_text` on line 43. The `asyncio.to_thread` variant is a negative.
