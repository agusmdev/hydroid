`Message(attachments=[{...}])` validates the dict into the nested `Attachment` model, which runs
`Attachment.must_exist` (`os.path.exists`) on the loop. hydroid looks at validators of the model
being constructed but not those of nested field types. Negatives: the default `[]` is not validated
(no nested construction), and the `asyncio.to_thread` construction runs off-loop.
