# ser_field_serializer

An async endpoint returns a pydantic model; FastAPI's `serialize_response` dumps it on the event
loop (async endpoints are not serialized in the threadpool). Pydantic calls `@field_serializer`
and `@model_serializer` methods during that dump, so blocking I/O inside them blocks the loop.
Nothing in user code calls these methods; the framework does, implicitly, at serialization time
(sibling of the open `computed_field_io` case, but a different pydantic hook).

Labels sit on the blocking call inside the sync serializer (the framework calls it on the loop),
as for computed fields. `Plain.upper` is a pure serializer: no diagnostic. The oracle confirms
both labeled lines block on the loop thread.
