# SynOS installer media

This media boots the SynOS UEFI loader and carries both firmware images.

- `payload/synos-bios.img`: write to a whole legacy-BIOS disk.
- `payload/synos-uefi.img`: write to a UEFI-capable removable disk.
- `tools/install-synos.sh`: checksum-verified release bundle helper.

The host helper requires an explicit `--yes` before it writes a target. Use
`--dry-run` to inspect the selected image and target first.
