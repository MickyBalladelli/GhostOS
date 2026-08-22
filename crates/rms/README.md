# ghostos-rms

`ghostos-rms` is the no-heap application data layer for GhostOS.

`RecordFile` wraps GhostFS RMS images with indexed or sequential create, read,
insert, update, and delete operations. Applications implement
`StructuredRecord` to encode typed records into caller-owned buffers. Resolve a
key selector to a `RecordLocation`, then acquire the matching `DlmLockRange`
through `DlmRecordLocks` before changing it. Inserts and deletes use a
whole-file update lock because they change record ordinals.

`Database` stores binary keys directly in the GhostFS B+tree namespace. A
`DatabaseTransaction` has a compile-time operation capacity and atomically
commits one CoW root. `MappedDatabase::visit_value` walks immutable data pages
directly for zero-copy reads.
