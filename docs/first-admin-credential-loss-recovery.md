# First administrator credential-loss recovery

This procedure applies when the first administrator account exists but none of
its registered passkeys, TPM credentials, or SSH keys can be used.

## Important boundary

GhostOS must not turn an existing account database back into first-run setup.
The built-in `RECOVERY` commands are only for interrupted setup while the
authorization database is absent:

```text
RECOVERY STATUS
RECOVERY RETRY
RECOVERY RESET
```

If `/system/security/authorization` exists, first-admin setup and these
commands are disabled. `RECOVERY RESET` must never be used to erase an account
database.

The current build does not yet ship an in-place credential-loss enrollment
command. Until trusted recovery-key tooling is available, use the final
section of this document: preserve the disk and restore a known-good backup or
reinstall. The steps below define the required operator ceremony for that
tooling; they are not a license to edit the account file by hand.

## Safe procedure

1. Keep the machine connected to the physical console. Do not boot an
   unverified kernel, installer, or recovery image.
2. Power the machine down and preserve a sector-for-sector copy of the system
   disk before attempting recovery.
3. Verify the signed recovery media and release checksums. The media procedure
   is documented in [`recovery-media.md`](recovery-media.md).
4. Use a trusted recovery key or other approved physical recovery proof. A
   lost administrator credential is not proof of recovery authority.
5. Inspect the account database. Preserve the existing administrator identity
   and account record; do not format the volume, delete
   `/system/security/authorization`, or create a replacement first admin.
6. Enroll one replacement public credential through the recovery tooling. Keep
   private passkey, TPM, and SSH material off the disk and out of logs.
7. Commit the credential change atomically, flush the GhostFS volume, and treat
   recovery as successful only after the durability operation reports success.
8. Remove the recovery media, reboot normally, log in with the replacement
   credential, and revoke the lost credential if it is still present.

## If trusted recovery proof is unavailable

Stop. GhostOS intentionally does not provide a password bypass or an
unauthenticated account reset. Restore a known-good full-system backup through
the verified recovery process, or reinstall after preserving user data. Do not
delete the account database as a shortcut.

Record the recovery event and rotate any secrets that may have been exposed
with the lost credential.
