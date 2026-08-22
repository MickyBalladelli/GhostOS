# Differential compatibility references

The compatibility suite lives in
[`virtual_machine/tests/differential_compatibility.rs`](../virtual_machine/tests/differential_compatibility.rs).
It compares real GhostOS paths with small independent references. A mismatch is
reduced by deterministic chunk and single-element deletion, then printed with
the minimized input and the decision text.

| Area | GhostOS path | Independent reference | Compatibility decision |
| --- | --- | --- | --- |
| Filesystem | `SynFs` latest-file writes, deletes, lookup, and reads | `BTreeMap<String, Vec<u8>>` latest-visible-content model | GhostFS keeps versioning and path validation; compare only latest visible state. |
| Network | Ring-3 fixed packet queue | bounded `VecDeque<Vec<u8>>` | FIFO order, admission, and queue depth must match; fixed capacity remains authoritative. |
| Terminal | Web terminal VT byte stream | bounded plain-byte VT model | Printable bytes, CR, LF, backspace, and wrap must match; unsupported controls remain GhostOS policy. |
| Firmware | BIOS POST and UEFI table initialization | published PC/UEFI signature and state facts | Keep externally visible firmware facts stable; implementation details may differ. |
| Serialization | generated RPC frame header encoder | literal big-endian frame encoder | Magic, version, fields, widths, and byte order are compatibility commitments. |

The references are intentionally not wrappers around GhostOS code. They are
small enough to audit, deterministic, and limited to the behavior named in the
decision column. Expanding a reference requires adding its compatibility rule
here at the same time.
