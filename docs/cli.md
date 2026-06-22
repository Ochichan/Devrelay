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
devrelay continue --source ../source --target ../target --config devrelay.local.toml --json
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

## Local Continue

`continue` is a local handoff simulator. It checkpoints the source workspace,
applies that snapshot to the target, and marks registered source/target
workspace states as inactive/active placeholders in local config.

## Exit Codes

- `0`: command completed successfully.
- `1`: command failed. Terminal output uses `error[CODE]: message`.

Use `--json-errors` to render failures as:

```json
{
  "error": {
    "code": "DR-...",
    "title": "Short title",
    "message": "human readable message",
    "detail": "specific failure detail",
    "safe_actions": ["non-destructive next step"],
    "diagnostic_id": "diag_..."
  }
}
```

Machine-readable success output is command-specific and enabled by each
command's `--json` flag.

Error codes are stable within their namespace: `DR-MANIFEST-*`, `DR-GIT-*`,
`DR-SNAPSHOT-*`, `DR-APPLY-*`, `DR-RECOVER-*`, and `DR-STORAGE-*`.
