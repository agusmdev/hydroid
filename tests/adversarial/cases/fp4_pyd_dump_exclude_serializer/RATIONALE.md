# fp4_pyd_dump_exclude_serializer

**Mechanism:** a field typed `Annotated[str, PlainSerializer(file_size)]` whose serializer stats
a file (`pathlib.Path.stat`). Endpoints dump the model with `exclude={"blob"}` or
`include={"title", "owner"}`: pydantic-core never serializes the left-out field, so its
serializer never runs.

**Why hydroid is likely wrong:** hydroid treats every `model_dump()` call as running the
`Annotated` serializers of all fields. The guide says it honors a literal
`response_model_exclude` for response serialization, but it does not apply the same reasoning to
literal `exclude=` / `include=` sets passed to `model_dump()` itself. It reports lines 30 and 36.

**Why the labels are right (runtime):**
- `pydantic/main.py` `BaseModel.model_dump` (line ~427): `include`: "A set of fields to include
  in the output", `exclude`: "A set of fields to exclude from the output"; both are passed to
  `self.__pydantic_serializer__.to_python(...)`, and pydantic-core's model/fields serializer
  filters fields before calling their field serializers (a filtered-out field's value is never
  serialized).
- Lines 30 and 36: oracle saw them run on the loop with no blocking call.
- Line 42: plain `model_dump()` runs `file_size` on the loop: oracle confirmed `pathlib.Path.stat`.
