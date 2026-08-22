# First boot and first administrator setup

This guide covers a new GhostOS system with no authorization database. Keep the
machine on a trusted physical console during setup.

## Boot

Build or obtain a boot image, then start it:

```sh
./scripts/build-bios-image.sh
./start-ghostos.sh
```

The launcher provisions `virtual_machine/state/system.raw` on first start.
The older `data.raw` file is only a data disk; it cannot save the first
administrator account.

The first-run shell prints:

```text
No administrator account exists.
GhostOS first-run setup mode
```

First-run commands are accepted case-insensitively. Use a valid username and
public credential material only:

```text
USERNAME admin
CREDENTIAL PASSKEY <public-material>
CONFIRM
```

The supported credential kinds are `PASSKEY`, `TPM`, and `SSH`. GhostOS stores
public credential material; never type or save a private key, seed, password,
or recovery secret in the system disk or serial logs.

Expected responses are:

```text
Administrator username saved.
Administrator credential saved.
Administrator account committed.
```

`CONFIRM` is the commit point. The account database is persisted atomically,
and the system leaves first-run mode only after the commit succeeds.

## Interrupted setup

If setup stops before the account database exists, inspect the pending state:

```text
RECOVERY STATUS
```

Use `RECOVERY RETRY` after fixing a failed durable write. Use
`RECOVERY RESET` only to clear an interrupted setup that has not committed an
authorization database. These commands must never be used to erase or replace
an existing account database.

## First login

After the administrator is committed, the normal shell appears. Log in from
the physical console:

```text
login
admin
passkey
<assertion-for-the-displayed-challenge>
```

On success, GhostOS prints `Login accepted.`. Verify the identity, then test the
lock cycle:

```text
whoami
logout
```

`whoami` must report `admin`. `logout` returns to the locked prompt; run
`login` again for a relogin. Failed attempts are rate-limited and repeated
failures cause a temporary login lockout.

## Safety rules

- Do not remove `/system/security/authorization` to force first-run mode.
- Do not use first-run recovery to bypass an existing administrator account.
- Preserve the system disk before recovery or credential-loss work.
- For a committed administrator with lost credentials, follow
  [`first-admin-credential-loss-recovery.md`](first-admin-credential-loss-recovery.md).
