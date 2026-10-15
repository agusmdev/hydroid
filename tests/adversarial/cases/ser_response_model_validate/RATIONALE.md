# ser_response_model_validate

Endpoints return plain dicts; FastAPI validates the return value against `response_model=` (or
the return annotation `-> FileOut`) in `serialize_response`, which for async endpoints runs on
the loop (`field.validate(...)` is only sent to the threadpool for sync endpoints). That
validation runs the model's `field_validator`s, so `requests.head` / `os.path.exists` inside them
block the loop - even though user code never constructs the model (unlike the
`pydantic_validators` case, where the endpoint builds the model explicitly).

Labels are on the blocking call inside the validator, the sync function the framework calls on
the loop. `NameOut.strip` is pure: no diagnostic. The oracle confirms both labeled lines.
