# Emergency recovery

Emergency recovery is for a system that cannot boot, cannot start its normal
services, has a damaged system disk, or needs a verified rollback. It restores
system availability. It is not an authentication bypass.

For a working system with lost administrator credentials, use
[`first-admin-credential-loss-recovery.md`](first-admin-credential-loss-recovery.md).
For an interrupted first-admin setup, use the `RECOVERY` commands in
[`first-boot.md`](first-boot.md).

## Recovery order

1. Stop normal writes. Do not keep retrying an unstable disk or booting
   unknown media.
2. Record the failure, current release, boot mode, target disk, and any error
   output. Keep logs and crash evidence unchanged.
3. Power the machine down if the disk, filesystem, or firmware state may be
   unsafe. Preserve a sector-for-sector copy of the system disk before repair.
4. Obtain the signed SynOS recovery release from a trusted source. Verify its
   release manifest, checksums, and recovery-media provenance on a separate
   trusted machine.
5. Choose the least destructive available action:
   - use a verified service or boot rollback when the persistent system is
     intact;
   - restore a verified backup when user state must be preserved;
   - restore the complete recovery image only when the boot image or system
     volume is not usable.
6. Boot the recovery media on the physical console. Match `bios` or `uefi` to
   the machine's firmware mode.
7. Copy user data off the target before a full-image restore. The restore
   replaces the target image; it does not merge files or account records.
8. Run the restore helper with `--dry-run`, inspect both image and target, then
   repeat with `--yes` only after the target is confirmed:

```sh
./tools/recover-synos.sh --mode bios --target <target-device> --dry-run
./tools/recover-synos.sh --mode bios --target <target-device> --yes
```

Use `--mode uefi` for a UEFI image. The helper checks an available checksum
file and refuses a restore without explicit `--yes`. Do not substitute a
different disk or image after the dry run.

9. Eject the recovery media before rebooting. Confirm that the restored image
   reaches the expected boot path, starts the required services, mounts SynFS,
   and presents the locked login prompt.
10. Log in with a known-good credential. Check identity, persistent files,
    service health, network state, and the release revision. Revoke lost
    credentials and rotate exposed secrets. Record the recovery event in the
    audit trail.

## Credential-loss emergency

If the administrator has lost every passkey, TPM credential, and SSH key:

- Do not run `RECOVERY RESET` against a committed system.
- Do not delete or edit `/system/security/authorization`.
- Do not create a replacement first administrator.
- Do not use a password, serial console, kernel shell, or recovery image as an
  authority proof.

Preserve the disk, verify trusted recovery authority, and follow the
[credential-loss procedure](first-admin-credential-loss-recovery.md). The
current recovery media can restore a known-good full system, but it cannot
enroll a replacement credential in an existing account database by itself.

## What recovery can and cannot do

Recovery can:

- replace a damaged or failed boot image with a verified release image;
- restore a verified bootable backup through its transactional restore path;
- return the machine to a known-good release after a failed update;
- revoke sessions when account, credential, or identity state is restored by
  the authenticated management service.

Recovery cannot:

- prove that the person holding recovery media owns an administrator account;
- bypass passkey, TPM, SSH, role, capability, or session authorization;
- turn an existing authorization database into first-run setup;
- safely preserve user changes when a complete image overwrite is used;
- make an unverified kernel, installer, backup, or recovery key trustworthy;
- recover data that was not copied to a verified backup or preserved disk
  image.

`RECOVERY STATUS`, `RECOVERY RETRY`, and `RECOVERY RESET` are only for
interrupted first-run setup while the authorization database is absent.
`RECOVERY RESET` is not a factory reset and must never be used as one.

## After an unsafe or incomplete attempt

Stop if checksum verification, boot verification, SynFS mounting, or service
health fails. Keep the original disk copy and the failed recovery media. Do
not repeatedly overwrite the target, downgrade to an unknown release, or
delete evidence to make the machine boot. Escalate with the preserved disk,
release revision, checksums, boot output, and recovery audit record.
