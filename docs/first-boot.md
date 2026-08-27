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

The VM opens a one-time local page automatically:

```text
Passkey setup and login: http://localhost:<port>/?code=<one-time-code>
```

Enter the administrator username and choose **Create passkey**. The browser and
authenticator create the private key; the local companion sends only the public
key to GhostOS. GhostOS commits the account and opens the initial shell session
automatically. Do not press Enter or type credentials in the terminal.

The local page is bound to loopback, protected by the one-time code, and uses
the browser's secure `localhost` WebAuthn context. Disable it with
`--no-passkey-web` only when using the manual TPM or SSH setup path:

```sh
./start-ghostos.sh --no-passkey-web
```

Expected responses after confirmation are:

```text
Administrator username saved.
Administrator credential saved.
Administrator account committed.
```

Answering `y` at the confirmation question is the commit point. The account
database is persisted atomically, and the system leaves first-run mode only
after the commit succeeds. The initial shell session opens automatically.

## Interrupted setup

If setup stops before the commit succeeds, start the machine again and repeat
the wizard. The next browser enrollment clears incomplete staged answers
before saving the new username and passkey.

## First login

The first administrator setup opens the shell directly after the passkey is
created. On later boots, the local page changes to **Unlock GhostOS**. Enter the
username and choose **Use passkey**; the browser touch/biometric prompt completes
the challenge automatically. Verify the identity, then test the lock cycle:

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
