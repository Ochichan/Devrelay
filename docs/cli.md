# DevRelay CLI

Last updated: 2026-06-24

The CLI is both a user surface and the safest way to exercise product
invariants alongside the desktop UI. Commands should keep terminal output
human-readable and `--json` output stable enough for tests and automation.

## Examples

```bash
devrelay manifest check devrelay_spec_bundle/devrelay.toml
devrelay manifest check devrelay_spec_bundle/devrelay.toml --json
devrelay project add . --manifest devrelay.toml --json
devrelay projects list --json
devrelay status --repo . --manifest devrelay.toml --json
devrelay checkpoint --repo . --manifest devrelay.toml --label "before refactor" --pin --json
devrelay snapshot list --project <project-id> --json
devrelay snapshot show <snapshot-id> --project <project-id> --json
devrelay snapshot export <snapshot-id> --project <project-id> --out snapshot.json --json
devrelay recover list --json
devrelay recover show <snapshot-id> --json
devrelay recover open <snapshot-id> --path ../recovery --register --name review --json
devrelay doctor environment --repo . --manifest devrelay.toml --json
devrelay doctor environment --repo . --manifest devrelay.toml --run-healthcheck --json
devrelay doctor secrets --repo . --manifest devrelay.toml --json
devrelay doctor resources --json
devrelay doctor anchor-health --json
devrelay doctor device-trust --json
devrelay environment status --project <project-id> --json
devrelay metrics export --project <project-id> --out metrics.json --json
devrelay continue --source ../source --target ../target --config devrelay.local.toml --json
devrelay apply --repo ../target --source . --snapshot .devrelay/snapshots/<id>.json --dry-run
devrelay apply --repo ../target --source . --snapshot snapshot.json --dirty-policy snapshot-and-fork --json
devrelay apply --repo ../target --source . --snapshot snapshot.json --dirty-policy new-workspace --json
```

## Agent Routing

The CLI may call the local agent for supported commands. Use `--direct` only as
a development escape hatch when testing core behavior without an agent.

```bash
devrelay --agent-socket <path> status --repo . --manifest devrelay.toml --json
devrelay --direct status --repo . --manifest devrelay.toml --json
```

Production UI should not duplicate CLI direct-mode behavior. UI state must come
from the agent.

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

UI copy should not expose these internal names. Use:

- `snapshot-and-fork`: "Preserve it as separate work and continue"
- `new-workspace`: "Open your incoming work in a new folder"
- `block`: "Cancel" or "Leave this device unchanged"

## Local Continue

`continue` is a local handoff simulator. It checkpoints the source workspace,
applies that snapshot to the target, and marks registered source/target
workspace states as inactive/active placeholders in local config.

This command is the CLI ancestor of the first desktop vertical slice. It is not
a substitute for real cross-device lease transfer and target preparation, but it
exercises the same dirty-target safety vocabulary.

## Environment Doctor

`doctor environment` inspects environment hydration blockers without changing
Git state. It checks declared profile compatibility, local adapter tools, local
secret provider mappings, and existing command trust records. Healthchecks run
only with `--run-healthcheck`; Dev Container image preparation also requires
`--allow-devcontainer-prepare`.

`environment status` reports the persisted hydration state for registered
projects and workspaces. It uses the local agent by default through
`environment.status`; `--direct` reads the same state files from
`$DEVRELAY_HOME/projects/<project-id>/hydration/`.

`doctor secrets` is the narrow secret mapping check. It reports required
manifest secrets, local provider mappings, and missing required mappings without
running environment healthchecks or materializing secret values.

`doctor resources` reports the configured resource profile, detected local
power/foreground context, active adjustments, and effective CPU/hash/network
limits. It warns when `resource_profile = "custom"` has no
`[resource_policy_limits]` table and therefore falls back to default custom
limits.

`doctor anchor-health` wraps anchor status as a doctor report. It checks whether
the device is configured as an anchor, verifies the local anchor metadata,
snapshot, CAS, and startup paths, and returns safe actions such as
`devrelay anchor init` when the anchor is not initialized.

`doctor device-trust` reports the local device identity, paired peer count,
registered devices, revocations, and key-rotation-required revocation warnings.
It returns pairing safe actions when no paired peer devices are registered.

## Local Metrics

`metrics export` writes a local JSON report. It uses the local agent by default
through `metrics.export`; `--direct` reads the same per-project metadata DBs and
hydration files without starting an agent.

The export is local-only and redacted by default. It includes aggregate counts
and durations derived from audit events, stored snapshots, handoff journals,
task-run metadata, and hydration records. It does not include source code,
snapshot objects, or raw logs. Use `--include-sensitive-paths` only for a local
debugging bundle that may contain configured local paths inside reason text.

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

## Safety Expectations

- Dirty target work is blocked or preserved before apply.
- Recovery defaults to a separate workspace or a clean target.
- Snapshot metadata and diagnostic output must not expose raw secret values.
- Agent-backed commands must preserve the same JSON shape as direct commands
  where compatibility is documented.
