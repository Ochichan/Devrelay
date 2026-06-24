# DevRelay User Guide

Last updated: 2026-06-24

This guide covers the current beta surface: local CLI commands, the local
agent, project registration, checkpointing, recovery, local handoff simulation,
environment checks, and safety policies. Real cross-device handoff and remote
task execution are still dogfood targets, so this guide calls out those limits
where they matter.

## Quick Start

Run these commands from a Git repository you want DevRelay to manage:

```bash
devrelay identity init
devrelay manifest check devrelay.toml
devrelay project add . --manifest devrelay.toml
devrelay status --repo . --manifest devrelay.toml
devrelay checkpoint --repo . --manifest devrelay.toml --label "before handoff"
devrelay recover list
```

If you are running from source, replace `devrelay` with:

```bash
cargo run -p devrelay-cli -- <command>
```

Preview service installation, or run the agent in the foreground while testing
the desktop app or agent-backed CLI routing:

```bash
devrelay agent install --dry-run
devrelay-agent --foreground
devrelay agent status
```

Use `--json` for machine-readable output and `--json-errors` for stable error
envelopes.

## Device Pairing

Initialize identity on each device:

```bash
devrelay identity init --json
devrelay identity show --json
```

Pairing currently uses explicit peer material rather than a polished QR or link
flow. Start a pairing session with peer device details from the other device or
from a future pairing offer:

```bash
devrelay pairing start \
  --peer-device-id <device-id> \
  --peer-name "<device name>" \
  --peer-signing-public-key <base64> \
  --peer-network-public-key <base64> \
  --peer-ephemeral-public-key <base64> \
  --json
```

Compare the displayed short authentication code out of band. If it matches on
both sides, confirm:

```bash
devrelay pairing confirm <pairing-id> --code <short-code> --json
```

List or revoke trusted devices:

```bash
devrelay devices list --json
devrelay device show <device-id> --json
devrelay device revoke <device-id> --reason "lost device" --json
```

Discovery can advertise or browse local services, but manual addresses are the
fallback when mDNS is unavailable:

```bash
devrelay discovery advertise --role peer --port <port> --dry-run --json
devrelay discovery browse --role peer --manual-address <host:port> --json
```

## Project Registration

Register each checkout that should participate in DevRelay:

```bash
devrelay project add /path/to/repo --manifest /path/to/repo/devrelay.toml --json
devrelay projects list --json
devrelay project show <project-id-or-name> --json
```

`project add` records the project ID, display name, device/workspace mapping,
and manifest-derived policies. Use a stable `project_id` in `devrelay.toml` when
the same project will exist on multiple devices.

Remove stale workspace mappings when a checkout is deleted or replaced:

```bash
devrelay workspace remove <workspace-id-or-path> --json
```

## Handoff

For local validation, use `continue` between two local checkouts:

```bash
devrelay continue \
  --source ../source \
  --target ../target \
  --dirty-policy block \
  --json
```

Use `--dry-run` first when testing a target. The command checkpoints the source,
applies that snapshot to the target, and exercises the same dirty-target policy
language used by the desktop handoff flow.

The desktop and agent expose the handoff state machine, but the project still
treats real two-device dogfood evidence as open. A handoff must not be
considered complete until target verification and lease commit have succeeded.

## Recovery

List recoverable snapshots and inspect one before opening it:

```bash
devrelay recover list --json
devrelay recover show <snapshot-id> --json
```

Open recovery into a new folder by default:

```bash
devrelay recover open <snapshot-id> \
  --path ../recovered-work \
  --register \
  --name "recovered review" \
  --json
```

Recovery into an existing dirty target is refused. Prefer a new folder unless
you have already confirmed the destination is disposable.

You can also export a portable snapshot metadata file:

```bash
devrelay snapshot export <snapshot-id> --project <project-id> --out snapshot.json --json
```

## Target Dirty State

DevRelay never silently overwrites dirty target work. Apply and local continue
use these policies:

- `block`: stop and leave the target unchanged.
- `snapshot-and-fork`: checkpoint the dirty target as pinned separate work, then
  apply the incoming snapshot.
- `new-workspace`: leave the dirty target unchanged and apply incoming work into
  a sibling recovery workspace.

Start with:

```bash
devrelay apply --repo ../target --source . --snapshot snapshot.json --dry-run
```

Choose `snapshot-and-fork` only when you want DevRelay to preserve the target's
current edits as a recoverable fork.

## Inactive Edits

After ownership moves away from a workspace, that workspace becomes inactive for
the canonical session. Edits made there are not discarded, but they cannot
advance canonical latest silently.

When inactive work is detected, preserve it as separate work and inspect it
through recovery or session commands:

```bash
devrelay sessions list --project <project-id> --json
devrelay session show <session-id> --project <project-id> --json
devrelay session fork <session-id> --project <project-id> --name "inactive work" --json
```

Treat inactive edits like a branch or fork that needs explicit review.

## Unsupported States

Checkpoint and apply safety blocks some Git states until they are resolved:

- unmerged index entries
- in-progress rebase, cherry-pick, revert, merge, or sequencer operations
- missing Git LFS objects needed by the snapshot
- corrupt or missing CAS chunks
- unsafe target paths for the current platform

Resolve the Git operation, make the working tree coherent, then retry
`checkpoint`, `continue`, or `apply`. See [unsupported-states.md](unsupported-states.md)
for the detailed policy.

## Security Model

DevRelay is local-first and conservative:

- Source code, snapshot objects, and raw logs stay local unless an explicit
  transfer or export asks for them.
- Devices must be paired before authenticated control-plane operations.
- mTLS and replay protection are the intended remote-control boundary.
- Command trust is scoped to project, device, and executable command hash.
- Secret materialization is local to the target environment and secret file
  targets are excluded from snapshots.
- Diagnostics, audit export, and metrics export redact sensitive paths by
  default.

If a device is lost or no longer trusted, revoke it:

```bash
devrelay device revoke <device-id> --reason "no longer trusted" --json
```

See [threat-model.md](threat-model.md) and
[data-loss-safety.md](data-loss-safety.md) for deeper security and safety
details.

## Backup

Do not treat backup anchor promotion as production-ready yet. The current safe
backup set is:

- your normal Git remotes
- `$DEVRELAY_HOME`, including per-project metadata and snapshot stores
- exported snapshot metadata files for important checkpoints
- any external secret provider data that DevRelay only references locally

Useful commands:

```bash
devrelay checkpoint --repo . --manifest devrelay.toml --label "backup point" --pin --json
devrelay snapshot list --project <project-id> --json
devrelay snapshot export <snapshot-id> --project <project-id> --out snapshot.json --json
devrelay anchor status --json
```

Keep identity material protected with local OS account permissions. `identity
recovery-export` is reserved for a future recovery format and should not be used
as a complete backup plan in this beta.

## Troubleshooting

Start with local state and doctors:

```bash
devrelay agent status --json
devrelay projects list --json
devrelay status --repo . --manifest devrelay.toml --json
devrelay doctor paths --repo . --manifest devrelay.toml --json
devrelay doctor line-endings --repo . --json
devrelay doctor git-performance --repo . --json
devrelay doctor environment --repo . --manifest devrelay.toml --json
devrelay environment status --project <project-id> --json
```

Run healthchecks only when you expect project bootstrap commands to execute:

```bash
devrelay doctor environment --repo . --manifest devrelay.toml --run-healthcheck --json
```

Export redacted support data locally:

```bash
devrelay diagnostics export --out diagnostics.json --json
devrelay metrics export --project <project-id> --out metrics.json --json
devrelay audit export --project <project-id> --out audit.json --json
```

Common blockers:

- Dirty target: rerun with `--dry-run`, then choose `block`,
  `snapshot-and-fork`, or `new-workspace`.
- Unsupported Git operation: finish or abort the operation before checkpointing.
- Environment not ready: run `doctor environment` and fix missing tools,
  command trust, or secret mappings.
- Agent unavailable: start the agent or use `--direct` only for local
  development/debugging.
- Missing snapshot data: keep the source or anchor available, or recover from a
  known exported snapshot and matching local store.
