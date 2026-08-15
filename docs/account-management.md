# Account, group, role, credential, session, and recovery commands

This guide covers the service-owned shell and the authentication management
API. Shell commands are case-insensitive. Run management commands after
logging in; an unauthenticated shell cannot change account state.

## Account commands

The interactive shell supports these commands:

```text
ACCOUNT LIST
ACCOUNT SHOW <username>
ACCOUNT CREATE <username>
ACCOUNT DELETE <username> CONFIRM
ACCOUNT ENABLE <username>
ACCOUNT DISABLE <username>
ACCOUNT RENAME <old> <new>
```

`ACCOUNT LIST` prints safe summaries. `ACCOUNT SHOW` prints the username,
state, local scope, and credential kind, but never credential material.

`ACCOUNT CREATE` makes a pending account. Add its first credential with
`CREDENTIAL ADD`; the account then becomes active. `ACCOUNT DELETE` needs the
literal `CONFIRM` argument and attempts to revoke that account's sessions.
SynOS refuses to delete or disable the last administrator.

`ACCOUNT ENABLE` needs at least one credential and changes a disabled account
to active. `ACCOUNT DISABLE` keeps the account and credentials but blocks login
and revokes its sessions. `ACCOUNT RENAME` normalizes the new name and keeps
the account's credential identity.

Usernames are 1–32 characters from letters, numbers, `.`, `_`, `-`, and `$`.
Names are case-insensitive. Reserved names such as `root`, `system`,
`operator`, and `guest` are rejected.

## Credential commands

```text
CREDENTIAL LIST <username>
CREDENTIAL ADD <username>
CREDENTIAL REMOVE <username> <id>
```

`CREDENTIAL LIST` shows credential IDs and kinds. It labels material as
`public-only`; private key material is never displayed or stored.

`CREDENTIAL ADD` prompts for:

```text
Credential type [PASSKEY/TPM/SSH]:
Public credential material (max 96 chars):
```

The supported kinds are `PASSKEY`, `TPM`, and `SSH`. An account can hold at
most four credentials. `CREDENTIAL REMOVE` accepts a decimal ID or a `0x`
hexadecimal ID and refuses to remove the last credential.

The lower-level management API also supports credential rotation, revocation,
labels, and expiration. Use `add_credential`, `remove_credential`,
`revoke_credential`, or `rotate_credential` with an authenticated session.
Changing credentials revokes that identity's active sessions in the API.

## Groups and roles

The service-owned shell does not currently accept `GROUP` or `ROLE` commands.
Use the authenticated `BootLoginService` management API:

```text
list_groups()
create_group(store, administrator, group_id, name, now)
add_group_member(store, administrator, group_id, identity, now)
remove_group_member(store, administrator, group_id, identity, now)
grant_group_right(store, administrator, group_id, right, now)
revoke_group_right(store, administrator, group_id, right, now)
assign_role(store, administrator, identity, role, now)
remove_role(store, administrator, identity, role, now)
```

The built-in roles are `administrator`, `operator`, `auditor`, and
`read-only`. Only an administrator session may change groups or roles. A
group grants a right only when both the identity is a member and the right is
present in the group. Group and role changes are persisted atomically.

## Session commands and operations

For the local shell:

```text
WHOAMI
LOGOUT
LOGIN
```

`WHOAMI` prints the current username. `LOGOUT` revokes the local shell session.
`LOGIN` starts a new login attempt after logout, timeout, or revocation. See
[`login.md`](login.md) for the credential prompts and remote login flows.

There is no interactive `SESSION` command in the service-owned shell. An
administrator uses these authenticated API operations:

```text
list_active_sessions(administrator, now)
terminate_session(administrator, target, now)
terminate_account_sessions(administrator, identity, now)
query_audit(administrator, query, now, visitor)
```

Session records include the handle, identity, credential, node, terminal,
authentication time, last activity, expiry, and revocation epoch. Account
disablement, deletion, credential changes, expiry, and explicit revocation
invalidate affected sessions.

## First-run recovery commands

Before an authorization database exists, the first-run shell accepts:

```text
RECOVERY STATUS
RECOVERY RETRY
RECOVERY RESET
```

`RECOVERY STATUS` reports the pending username, credential, and sync state as
bounded hexadecimal values. `RECOVERY RETRY` retries a failed durable write.
`RECOVERY RESET` clears only interrupted setup that has not committed an
authorization database.

These commands cannot reset an existing account database. If a committed
administrator loses every credential, do not delete
`/system/security/authorization`. Follow
[`first-admin-credential-loss-recovery.md`](first-admin-credential-loss-recovery.md).
The current shell has no password bypass or committed-account reset command.

The authentication API has a separate trusted recovery flow:

```text
begin_credential_recovery(...)
complete_credential_recovery(...)
complete_physical_credential_recovery(...)
```

It requires the configured trusted recovery identity and credential, an
unexpired recovery challenge, and an atomic state commit. Recovery revokes the
target identity's old sessions.
