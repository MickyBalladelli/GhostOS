# Release upgrade compatibility

Every release needs a two-way compatibility proof against the previous
release. The proof is a JSON file passed as `--upgrade-compatibility` (or
`GHOSTOS_UPGRADE_COMPATIBILITY` for the release gate).

The validator checks that:

- the release and previous revisions are distinct and the release revision is
  the current Git revision;
- every boundary has valid upgrade and rollback version ranges;
- the new release accepts the previous version and rollback accepts the new
  version;
- version changes use `read-convert` compatibility; and
- both directions have passed JSON evidence under the release evidence root.

Minimal shape:

```json
{
  "schema": "ghostos-upgrade-compatibility",
  "schema_version": 1,
  "release": {"version": "0.2.0", "revision": "NEW_REVISION"},
  "previous": {"version": "0.1.0", "revision": "OLD_REVISION"},
  "rollback": {"supported": true, "target_revision": "OLD_REVISION"},
  "artifacts": [
    {
      "name": "snapshot",
      "previous_version": 1,
      "release_version": 2,
      "upgrade": {
        "minimum_peer_version": 1,
        "maximum_peer_version": 2,
        "mode": "read-convert"
      },
      "rollback": {
        "minimum_peer_version": 1,
        "maximum_peer_version": 2,
        "mode": "read-convert"
      },
      "migration": {"id": "snapshot-v1-to-v2"},
      "evidence": {
        "upgrade": ["compatibility/snapshot-upgrade.json"],
        "rollback": ["compatibility/snapshot-rollback.json"]
      }
    }
  ]
}
```

Validate it directly with:

```sh
python3 scripts/validate-upgrade-compatibility.py \
  --manifest build/release/upgrade-compatibility.json \
  --evidence-dir build/test-evidence/<run-id>
```

The release gate and VM package command run the same validator before release
output is accepted.
