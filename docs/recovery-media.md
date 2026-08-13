# SynOS recovery media

Recovery media contains the last release BIOS and UEFI images plus the
operator restore helper. Restore writes a complete boot image, so it replaces
the target image contents. Copy user data off the target before restoring.

Use `tools/recover-synos.sh --mode bios|uefi --target PATH --dry-run` to inspect
the operation. Add `--yes` only after checking the target path.
