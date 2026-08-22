# 10. The Shell and OpenVMS Ideas

GhostOS uses a native shell to make system state inspectable and automation predictable. It borrows useful OpenVMS ideas without copying the host shell model.

## Typed commands

Commands have typed positional arguments and qualifiers. The parser validates the complete invocation before an executor runs.

```text
SHOW CLUSTER/HEALTH
LIST CLUSTERS /FEDERATED /LIMIT=20 /PAGE=2
REMOVE NODE node-7 /FORCE /CONFIRM
```

Two-word and canonical hyphenated forms can be equivalent. A qualifier is data, not text pasted into a command string.

## Structured output

Pipeline stages exchange `StructuredOutput` records. The terminal can render those records as human-readable lists or JSON. This means the following idea is safe:

```text
SHOW CLUSTER/MEMBERS | SELECT NAME,STATE | FORMAT JSON
```

The exact command set evolves, but the rule stays: parse once, validate once, pass typed records.

Background commands and complete pipelines enter a bounded system job queue
with priorities, dependencies, retry limits, worker leases, cancellation, and
lost-worker recovery. Command dispatch does not wait for the whole job to
finish; it returns a stable job identity that can be inspected later.

## Filesystem workflows

Typical commands include:

```text
DIRECTORY /DATA
CREATE /DATA/notes
TYPE /DATA/notes
SET DEFAULT /DATA
LINK /DATA/notes /DATA/today
DELETE /DATA/today
```

The shell checks path syntax and submits the operation with the user’s capabilities. The filesystem service remains the final authority.

## The editor

`EDIT` is a bounded UTF-8 full-screen editor. It edits a selected GhostFS version and saves as a new version. It supports cursor movement, selection, copy/cut/paste, scrolling, resizing, conflict detection, and terminal restoration.

The memorable control set is:

```text
Ctrl-S  save
Ctrl-Z  save and exit
Ctrl-X  discard and exit
Escape  command mode
I       insert
S       save
E       save and exit
Q       quit
```

If another writer changes the file first, the editor reports a conflict. It does not silently overwrite a newer version.

## Cluster administration

The cluster command registry exposes stable routes:

```text
SHOW CLUSTER
SHOW CLUSTER/MEMBERS
SHOW CLUSTER/TOPOLOGY
SHOW CLUSTER/HEALTH
SHOW CLUSTER/RESOURCES
CREATE CLUSTER compute /DESCRIPTION="local fabric" /QUORUM=3
JOIN CLUSTER /INVITATION="token" /ENDPOINT="10.0.0.2"
```

Destructive actions require `/CONFIRM`; `/FORCE` adds a stronger authorization check and never replaces confirmation. `/DRY_RUN` is used for mutable configuration where supported.

## Permission classes

The shell recognizes roles such as administrators, operators, auditors, read-only users, and node owners. The role is not enough by itself. The final check also sees the capability, object owner, cluster epoch, quorum state, and operation-specific confirmation.

## Statuses users can act on

Common outcomes include:

- normal;
- invalid argument;
- access denied;
- confirmation required;
- quorum lost;
- cluster partitioned;
- protocol mismatch;
- stale state;
- node unsafe;
- reconciliation required.

These statuses make operations teachable. A user can fix “confirmation required.” A service can retry “busy.” An operator can fence “node unsafe.”
