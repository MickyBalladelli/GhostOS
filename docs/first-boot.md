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

The first-run wizard prints:

```text
No administrator account exists.
GhostOS first-run setup mode
```

Answer each question. Use a valid username and public credential material
only. For a passkey, enter the registered COSE ES256 public key as hexadecimal
CBOR bytes:

```text
Administrator username: admin
Credential type [PASSKEY/TPM/SSH] (PASSKEY): passkey
Public credential material (passkey COSE key as hex): <cose-key-hex>

Username: admin
Credential type: PASSKEY
Material: <cose-key-hex>
Create this administrator account? [y/N]: y
```

Press Enter at the credential type question to accept the default `PASSKEY`.
The supported credential kinds are `PASSKEY`, `TPM`, and `SSH`. GhostOS stores
public credential material; never type or save a private key, seed, password,
or recovery secret in the system disk or serial logs.

Expected responses after confirmation are:

```text
Administrator username saved.
Administrator credential saved.
Administrator account committed.
```

Answering `y` at the confirmation question is the commit point. The account
database is persisted atomically, and the system leaves first-run mode only
after the commit succeeds, then the normal shell starts.

## Interrupted setup

If setup stops before the commit succeeds, start the machine again and repeat
the wizard with identical answers; pending answers stay valid until the commit
succeeds. Changing an answer is rejected until the original answers are
repeated or the system disk is reprovisioned.

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

- Do not remove `/system/security/authorization` to force first-run setup.
- Do not use first-run setup to bypass an existing administrator account.
- Preserve the system disk before recovery or credential-loss work.
- For a committed administrator with lost credentials, follow
  [`first-admin-credential-loss-recovery.md`](first-admin-credential-loss-recovery.md).
