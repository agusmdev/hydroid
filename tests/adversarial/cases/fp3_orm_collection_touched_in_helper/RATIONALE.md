# Lazy collection already loaded in the threadpool helper

`load_project` runs in the threadpool (`run_in_threadpool`) and touches `project.members`
(`len(project.members)`) before returning, a common way to "preload" a relationship while still
off the loop. The endpoint later iterates `project.members` on the loop: the collection is already
in the instance dict, so no SQL runs.

hydroid reports line 67 (`project.members` in the endpoint) as a lazy-load `Session.execute` on the
loop. It does not see that the same relationship was loaded on the same object in the offloaded
helper. `project.labels` (line 76) is never touched in the helper and really lazy-loads on the
loop - labelled, and the oracle confirms it.

## Why the labels are right

- First access in the helper goes through the lazy loader in the worker thread and stores the
  collection in `instance.__dict__`.
- `InstrumentedAttribute.__get__` (SQLAlchemy 2.0.54, `orm/attributes.py:555-569`): when
  `self.key in dict_`, it returns `dict_[self.key]` without calling `impl.get` (the lazy loader).
- `expire_on_commit=False`, no commit/expire between the helper and the endpoint, and the session
  stays open for the request (generator dependency), so the loaded state is kept.

The oracle: line 67 ran on the loop and nothing blocked; line 76 blocked in `Session.execute`.
