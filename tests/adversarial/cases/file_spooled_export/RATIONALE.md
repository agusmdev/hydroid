# file_spooled_export

An `async def` CSV export builds its body in a `tempfile.SpooledTemporaryFile` it creates itself
(the same class FastAPI uses behind `UploadFile.file`). `.write()` / `.read()` on it are file I/O:
once the content passes `max_size` it rolls over to a real `TemporaryFile` on disk, and every later
write/read goes through a real file object.

hydroid reports `UploadFile.file.read()` because starlette types that attribute as `BinaryIO`
(`typing.IO.read` is a sink), but a directly constructed `SpooledTemporaryFile` resolves to
`tempfile.SpooledTemporaryFile.write/read`, which are not treated as blocking: all three labeled
lines are missed.

Runtime: the oracle records `typing.IO.write` on the header write, the row writes and
`typing.IO.read` on the final read, all on the loop thread. The `io.StringIO` endpoint is the
in-memory near-miss and records nothing.
