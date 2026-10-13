`ImportJob.source` is `Annotated[str, AfterValidator(file_must_exist)]` (via a type alias). Pydantic
runs the AfterValidator during `ImportJob(...)` in the async endpoint, so `os.path.isfile` runs on
the loop. hydroid handles `@field_validator` methods but not validators attached through `Annotated`
metadata on model fields. Negatives: `model_construct` (no validation) and a `def` endpoint.
