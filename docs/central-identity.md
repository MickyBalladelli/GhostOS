# Central identity and VM fleet

GhostOS now has a transport-independent directory authority in
`ghostos-auth::Directory`. It is a service boundary, not a requirement that
the directory become a special VM. Run it as one trusted service with durable
storage, or place it behind an existing control-plane service.

The directory owns one user record per identity. A record contains a stable
`IdentityId`, username, groups and roles, credential labels, public passkey
material, and revocation state. It never stores a passkey private key. One
credential record can therefore authenticate the same user on every enrolled
VM.

## VM enrollment

An image or VM creates a `NodeJoinRequest` with its node ID and the fleet
bootstrap key. The directory accepts a fresh, authenticated request once and
returns a `DirectoryTrustAnchor`. The anchor contains the directory issuer,
the node audience, and the current revocation and policy generations.

Node removal uses `Directory::revoke_node`. It invalidates the node anchor and
all tokens issued under the old generations. Re-enrollment creates a fresh
anchor. Copying `/system/security/authorization` between VMs is not a join
operation and must not be used.

## Login and tokens

The directory login path is:

1. The VM asks `Directory::begin_login` for a one-shot credential challenge.
2. The authenticator signs the challenge. Only the public credential remains
   in the directory.
3. `Directory::complete_login` verifies the proof and creates a short-lived
   directory session.
4. `Directory::issue_token` returns an HMAC-authenticated identity token
   bound to the identity, credential, directory issuer, target VM, completed
   login challenge, expiry, revocation epoch, policy generation, and nonce.
5. The VM uses `DirectoryTokenVerifier` once. Replays, wrong audiences,
   wrong challenges, stale generations, and expired tokens fail.

The current fixed-size token primitive uses a `CapabilityKey` HMAC shared with
the enrolled verifier. Provision that key only to trusted GhostOS nodes; a
hostile node must not be treated as a verifier-only boundary until the token
primitive is upgraded to an asymmetric directory signing key.

The token contains identity claims and right identifiers only. It does not
contain a filesystem object, capability handle, or unrestricted kernel right.
The node maps verified claims to an `ExecutionPersona`, then applies its own
kernel capability policy.

The default fleet WebAuthn values are stable:

```text
RP ID:  login.ghostos.local
Origin: https://login.ghostos.local
```

All VM login pages must use the same HTTPS origin and RP ID. A local companion
page using `http://localhost` remains a separate local-development ceremony;
it must not be presented as the fleet directory ceremony.

## Offline and recovery behavior

After an online token is verified, `DirectoryOfflineCache` can retain its
claims only until the token expiry or the configured offline limit, whichever
comes first. The caller must provide the current revocation epoch and policy
generation on every offline authorization. A network partition never extends
the cache lifetime.

Use `DirectoryBootstrapPolicy` to make the node choice explicit:

- central directory required for normal login;
- bounded offline login during a partition;
- optional local break-glass credential for recovery.

`DirectoryBootstrapPolicy::select_mode` distinguishes central enrollment,
central login, offline login, local break-glass, first boot, directory
unavailable, and recovery-required states. A directory cannot revoke the last
active administrator, so recovery and credential rotation have a safe order:
add the replacement credential or administrator first, then revoke the old
one.

The local `/system/security/authorization` database remains the break-glass
and migration store. It is not the central directory database. Credential
rotation, identity revocation, and group changes must call
`mark_policy_changed` or a revocation method so cached claims stop working.
