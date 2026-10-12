FastAPI serializes the model an async endpoint returns on the event loop (serialize_response runs
field.serialize directly when the endpoint is a coroutine). Pydantic runs @computed_field
properties during serialization, so their blocking calls run on the loop. hydroid does not treat
computed fields of a returned model (or a response_model) as code that runs on the loop.
