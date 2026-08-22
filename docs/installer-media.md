# GhostOS installer media

This media boots the GhostOS UEFI loader and carries both firmware images.

- `payload/ghostos-bios.img`: write to a whole legacy-BIOS disk.
- `payload/ghostos-uefi.img`: write to a UEFI-capable removable disk.
- `tools/install-ghostos.sh`: checksum-verified release bundle helper.

The host helper requires an explicit `--yes` before it writes a target. Use
`--dry-run` to inspect the selected image and target first.
