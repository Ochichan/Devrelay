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
devrelay recover list --json
devrelay recover show <snapshot-id> --json
devrelay recover open <snapshot-id> --path ../recovery --register --name review --json
devrelay apply --repo ../target --source . --snapshot .devrelay/snapshots/<id>.json --dry-run
devrelay apply --repo ../target --source . --snapshot snapshot.json --dirty-policy snapshot-and-fork --json
devrelay apply --repo ../target --source . --snapshot snapshot.json --dirty-policy new-workspace --json
```

## Snapshot Store

`checkpoint` stores synthetic snapshot refs in:

```text
$DEVRELAY_HOME/projects/<project-id>/snapshots.git
```

Snapshot metadata is persisted in the per-project SQLite database under the same
project data directory. Use `--out <path>` on `checkpoint`, or `snapshot export`,
to write a portable metadata JSON file.

## Dirty Apply Policies

`apply` defaults to `--dirty-policy block`. `snapshot-and-fork` stores a pinned
backup snapshot of the dirty target before cleaning and applying. `new-workspace`
leaves the dirty target unchanged and applies into a sibling recovery workspace.

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
