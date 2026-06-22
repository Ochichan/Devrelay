# DevRelay CLI

## Examples

```bash
devrelay manifest check devrelay_spec_bundle/devrelay.toml
devrelay manifest check devrelay_spec_bundle/devrelay.toml --json
devrelay status --repo . --manifest devrelay.toml --json
devrelay checkpoint --repo . --manifest devrelay.toml --json
devrelay apply --repo ../target --source . --snapshot .devrelay/snapshots/<id>.json --dry-run
devrelay apply --repo ../target --source . --snapshot .devrelay/snapshots/<id>.json --json
```

## Snapshot Output

`checkpoint` writes snapshot metadata to:

```text
.devrelay/snapshots/<snapshot-id>.json
```

Use `--out <path>` to choose a different file.

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
