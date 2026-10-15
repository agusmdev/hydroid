# Plugin registry filled by __init_subclass__

`Notifier.__init_subclass__` registers each subclass under its `channel=` class keyword in
`Notifier.registry: ClassVar[dict[str, type[Notifier]]]`. The route picks a class from the
registry (falling back to the `"log"` notifier), instantiates it and calls `send` inline.
`send` is only defined on the subclasses (`WebhookNotifier.send` -> `requests.post`,
`LogNotifier.send` -> `time.sleep`), not on the base, so resolving the call needs the subclasses
registered by `__init_subclass__`; hydroid reports nothing.

Runtime: with the sample input `channel="1"` the lookup misses and falls back to
`LogNotifier`, whose `send` calls `time.sleep` on the loop - confirmed. Labeled `*` because a
correct static answer may name either subclass's blocking call.
