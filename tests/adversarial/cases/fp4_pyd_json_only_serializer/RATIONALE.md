# fp4_pyd_json_only_serializer

**Mechanism:** an `Annotated[str, PlainSerializer(f, when_used="json")]` field. Its serializer
does a reverse DNS lookup (`socket.gethostbyaddr`), but only when the model is serialized to
JSON. `peer.model_dump()` (python mode, the default) skips it.

**Why hydroid is likely wrong:** hydroid records Annotated serializers of model fields and treats
any `model_dump()` / `model_dump_json()` as running them, without looking at `when_used`. It
reports line 31 (`peer.model_dump()`), which never calls `host_label`.

**Why the labels are right (runtime):**
- `pydantic_core/core_schema.py` (`WhenUsed`, line ~299): `'json'` means "use when serializing
  to JSON". `pydantic/functional_serializers.py` (`PlainSerializer.when_used`, lines 46-52,
  passed to `plain_serializer_function_ser_schema(..., when_used=self.when_used)` line 84).
  `model_dump()` defaults to `mode='python'`, so pydantic-core's serializer skips the function
  and emits the raw string.
- `/peers/raw` (line 31): oracle saw the line run on the loop with no blocking call.
- `/peers/export` (line 38): `model_dump_json()` runs `host_label` on the loop: oracle confirmed
  `_socket.gethostbyaddr`.
- `/peers/sync`: `def` endpoint, threadpool.
