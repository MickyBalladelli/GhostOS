# Local and remote login

GhostOS supports these login paths:

- local physical-console login with a passkey
- local physical-console login with a TPM-backed credential
- remote login with an SSH public key or a browser passkey

GhostOS stores public credential material only. Never put a private key, passkey
seed, TPM secret, password, or recovery secret on the system disk or in a
serial log.

## Local physical-console login

After the first administrator account exists, the login service keeps the
terminal locked. At the physical console, type:

```text
login
```

The service asks for the username and credential type:

```text
Username: admin
Credential [passkey/tpm]: passkey
```

For a passkey, touch the authenticator when asked. The service displays a
one-shot challenge and asks for the assertion as hexadecimal data:

```text
Touch your passkey and paste the assertion as hex.
Challenge: <displayed-challenge>
Passkey assertion: <assertion-hex>
```

For a TPM credential, choose `tpm`. Present the TPM-backed credential and
provide its quote as hexadecimal data:

```text
Credential [passkey/tpm]: tpm

Present your TPM-backed credential and paste the quote as hex.
Challenge: <displayed-challenge>
TPM quote: <quote-hex>
```

Credential input is not echoed. The username accepts 1–32 letters, numbers,
`.`, `_`, `-`, and `$`. A blank credential type means `passkey`.

On success, the service prints `Login accepted.` and the shell changes to the
normal `$` prompt. Check the identity:

```text
WHOAMI
```

Lock the console again with:

```text
LOGOUT
```

Failed attempts are rate-limited. Five failures cause a temporary lockout.
The session expires after 15 minutes and becomes idle after 5 minutes without
activity. `LOGIN` starts a new login attempt after logout, timeout, or session
revocation.

## Remote SSH login

SSH uses a configured `SSH` public-key credential for the account. Enroll the
public key during first-boot setup or with the authorized credential-management
flow. Keep the matching private key on the client only.

The configured SSH transport performs this flow:

1. The client starts an SSH connection and sends the username and public key.
2. GhostOS checks that the account, node, credential, and policy allow the key.
3. GhostOS creates a one-shot challenge bound to the SSH exchange hash, public
   key, node, device, and expiry.
4. The client signs the challenge with the private key.
5. GhostOS verifies the signature, creates an expiring session, and opens
   `ghostos-shell` with the session's shell capability.

Use the SSH client and endpoint configured by the system operator. The
transport adapter owns host, port, and key-file settings; this repository does
not define one universal SSH address. A normal client shape is:

```sh
ssh -i <private-key> <username>@<ghostos-host>
```

The server must never ask for the private key. A failed key, unknown account,
expired credential, expired challenge, replay, or revoked session is rejected.
Open SSH sessions are revalidated during terminal I/O and close when their
identity or capability changes.

## Remote browser login

Browser administration uses a WebAuthn passkey over HTTPS. It is not a
username-and-password login.

The browser and gateway perform this flow:

1. The gateway starts a login ceremony for the configured username, passkey,
   login node, device, relying-party ID, and HTTPS origin.
2. The browser calls the authenticator. The user must provide presence and
   verification, such as a touch plus device PIN or biometric check.
3. The browser returns the authenticator data, `clientDataJSON`, and signature.
4. The gateway verifies the COSE signature, challenge, ceremony type, origin,
   relying-party hash, credential, expiry, and authenticator counter.
5. GhostOS creates the remote session. The gateway may then issue only the
   configured subset of capabilities for that session.

Challenges are one-shot and live for at most 120 seconds. Remote sessions live
for at most 15 minutes. Remote capability tokens live for at most 5 minutes,
are tied to the authenticated device and session, and cannot grant map,
create, delegate, or revoke rights.

Use the exact HTTPS origin and relying-party identity configured by the
operator. Do not bypass certificate checks, use an HTTP copy of the service,
or approve an authenticator prompt whose origin or device is unexpected.

## Login failures and recovery

Login failures do not reveal whether an account exists. Repeated failures
trigger rate limiting and temporary lockout. Logout, timeout, account changes,
credential revocation, and session revocation return the shell to the locked
prompt or close the remote terminal.

If every administrator credential is lost, do not delete
`/system/security/authorization` and do not use first-run recovery to create a
replacement administrator. Follow
[`first-admin-credential-loss-recovery.md`](first-admin-credential-loss-recovery.md).
