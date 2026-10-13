`Report.title` carries `Annotated[str, PlainSerializer(slow_label)]`. An explicit `r.model_dump()`
in the async endpoint runs `slow_label` (`time.sleep`) on the loop. The endpoint returns a plain
dict so FastAPI's response serialization does not re-run it. hydroid does not follow serializers
declared through `Annotated` metadata from an explicit `model_dump()`. Negatives: attribute access
and a `def` endpoint.
