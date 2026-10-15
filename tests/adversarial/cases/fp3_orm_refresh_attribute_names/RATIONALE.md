# Lazy collection loaded by `Session.refresh(..., attribute_names=[...])` in the worker thread

`load_project` runs in the threadpool (`run_in_threadpool`). After `db.get`, it calls
`db.refresh(project, attribute_names=["members"])`, which loads the lazy `members` collection
right there, off the loop. The endpoint then iterates `project.members` on the loop: no SQL.

hydroid reports line 67 (`project.members`) as a lazy-load `Session.execute` on the loop: it does
not account for the explicit refresh of that relationship. `project.labels` (line 76) is NOT
refreshed and really lazy-loads on the loop - labelled, and the oracle confirms it.

## Why the labels are right

- SQLAlchemy 2.0.54 `orm/session.py:3095-3107` (`Session.refresh` docstring): relationship
  attributes that use the lazy `select` loader "will also load **if they are named explicitly in
  the attribute_names collection**, emitting a SELECT statement for the attribute using the
  `immediate` loader strategy" (since 2.0.4).
- `InstrumentedAttribute.__get__` (`orm/attributes.py:555-569`): if the key is already in the
  instance `__dict__`, it returns `dict_[self.key]` and never calls `impl.get` / the lazy loader.
- `expire_on_commit=False` and no commit in between, so nothing expires the loaded collection.

The oracle: line 67 ran on the loop and nothing blocked; line 76 blocked in `Session.execute`.
