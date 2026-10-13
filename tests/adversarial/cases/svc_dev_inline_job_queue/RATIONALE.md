A `@job` decorator wraps task functions into `Job` objects (`func` stored on the instance), and
services call `task.enqueue(...)`. Like RQ/Celery/arq helpers in many FastAPI codebases, the
enqueue helper runs the job inline when `settings.run_jobs_inline` (development, the default
here) and pushes to a broker otherwise.

Why hydroid likely misses it: the callable reaches `self.func` through a decorator application
(`sync_customer_to_crm = job(sync_customer_to_crm)`), not an explicit constructor call with a
function argument, so the call `self.func(*args)` inside `Job.enqueue` is not linked to the
decorated function. Note the precision trap: `warm_cache.enqueue(...)` is another `Job` whose
`func` only prints - it must stay clean.

Runtime: `POST /customers` -> `CustomerService.create` (async) -> `sync_customer_to_crm.enqueue`
-> `self.func` -> `requests.post` on the loop thread (APP_ENV defaults to development). The
oracle confirms it at the labeled line. `/customers/batch` offloads the same `enqueue` with
`asyncio.to_thread` (negative).
