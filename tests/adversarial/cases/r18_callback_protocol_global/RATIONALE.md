# r18_callback_protocol_global

A callback Protocol (`FeatureFlagSource` with only `__call__`) types a module-level strategy
variable: `flag_source: FeatureFlagSource = load_flags_from_disk`. With the declared type, the
call `flag_source(tenant)` resolves statically to `FeatureFlagSource.__call__`, whose body is `...`.
The plain function assigned to it is not a class implementing the Protocol, so no override search
finds it. (Without the annotation hydroid reports the line, so the Protocol type is what hides it.)

At runtime `flag_source` is the function `load_flags_from_disk`, which calls `open()` on the loop
thread. Negative: the same callable offloaded with `asyncio.to_thread`.
