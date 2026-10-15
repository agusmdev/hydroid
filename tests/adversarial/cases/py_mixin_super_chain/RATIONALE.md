Cooperative mixins: `ItemRepository(AuditMixin, TimestampMixin, Repository)`. `repo.save()`
resolves to `AuditMixin.save`, whose `super().save()` follows the MRO of `ItemRepository`
(AuditMixin -> TimestampMixin -> Repository), not of `AuditMixin` (whose only base is `object`).
So it ends in `Repository.save` -> `sqlite3.connect`. Statically, `super()` inside the mixin
resolves against `AuditMixin`'s own bases and finds nothing, so the chain is cut and the call is
missed. Runtime confirms `_sqlite3.connect` on the loop at line 46. Negative: `FakeItemRepository`
uses the same mixin over an in-memory base: its MRO ends in `MemoryRepository.save`, nothing blocks.
