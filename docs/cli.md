# DevRelay CLI

## Examples

```bash
devrelay manifest check devrelay_spec_bundle/devrelay.toml
devrelay manifest check devrelay_spec_bundle/devrelay.toml --json
devrelay status --repo . --manifest devrelay.toml --json
devrelay checkpoint --repo . --manifest devrelay.toml --label "before refactor" --pin --json
devrelay snapshot list --project <project-id> --json
devrelay snapshot show <snapshot-id> --project <project-id> --json
devrelay snapshot export <snapshot-id> --project <project-id> --out snapshot.json --json
devrelay apply --repo ../target --source . --snapshot .devrelay/snapshots/<id>.json --dry-run
devrelay apply --repo ../target --source . --snapshot .devrelay/snapshots/<id>.json --json
```

## Snapshot Store

`checkpoint` stores synthetic snapshot refs in:

```text
$DEVRELAY_HOME/projects/<project-id>/snapshots.git
```

Snapshot metadata is persisted in the per-project SQLite database under the same
project data directory. Use `--out <path>` on `checkpoint`, or `snapshot export`,
to write a portable metadata JSON file.

## Exit Codes

- `0`: command completed successfully.
- `1`: command failed. Terminal output uses `error[CODE]: message`.

Use `--json-errors` to render failures as:

```json
{
  "error": {
    "code": "DR-...",
    "message": "human readable message"
  }
}
```

Machine-readable success output is command-specific and enabled by each
command's `--json` flag.
