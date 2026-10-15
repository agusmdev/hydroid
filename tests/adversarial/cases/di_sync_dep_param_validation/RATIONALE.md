# Parameters of a sync dependency are validated on the loop

A sync (`def`) dependency body runs in the threadpool, but FastAPI's `solve_dependencies`
extracts and validates that dependency's own parameters (query, body...) with pydantic *before*
calling `run_in_threadpool`, on the event loop. So user code attached to those parameters -
an `AfterValidator` on a query parameter, a `field_validator` on a body model - runs on the loop.

Labels sit on the blocking calls inside the validators (sync functions the framework calls on
the loop). The blocking calls inside the dependency bodies (`time.sleep`, `os.path.exists`) are
negatives: those bodies really do run in the threadpool. The runtime oracle confirms both labels
and nothing else.

Likely hydroid miss: the dependency is sync, so everything reachable from it is considered off
the loop, and validators triggered by FastAPI's request parsing are not modeled as loop code.
