# Install, Update, And Removal

Last updated: 2026-06-24

This document defines the current dev and beta installation paths. It covers
the source-built CLI and agent that exist today, plus the WSL operating model.
Signed macOS, Linux, and Windows installers are still release-candidate work and
are tracked separately in `docs/north-star-checklist.md`.

## Channels

Dev channel:

- Built from a local Git checkout.
- Uses `cargo run`, `cargo build`, or locally copied binaries.
- Does not auto-update.
- May use unreleased schemas while the workspace is under active development.
- Must run doctors before participating in real handoff dogfood.

Beta channel:

- Reserved for signed artifacts intended for daily use on private machines.
- Requires published checksums, release notes, and migration notes.
- Must preserve the metadata and snapshot schema support windows in
  `docs/release-update-policy.md`.
- Must not run destructive migrations without a pre-migration backup.

## Dev CLI And Agent

Build the current CLI and agent from the repository root:

```bash
cargo build --release -p devrelay-cli -p devrelay-agent
export PATH="$PWD/target/release:$PATH"
devrelay --help
```

Preview the per-user agent service before writing a service file:

```bash
devrelay agent install \
  --dry-run \
  --agent-bin "$PWD/target/release/devrelay-agent"
```

Install the per-user agent service:

```bash
devrelay agent install \
  --agent-bin "$PWD/target/release/devrelay-agent"
```

The command prints the platform-specific follow-up command:

- macOS: `launchctl bootstrap gui/$(id -u) <service-path>` and
  `launchctl enable gui/$(id -u)/com.devrelay.agent`
- Linux: `systemctl --user daemon-reload` and
  `systemctl --user enable --now devrelay-agent.service`

Check service state with:

```bash
devrelay agent status --json
```

## Desktop Dev Build

The desktop app can be built from source for local validation:

```bash
cd apps/desktop
npm install
npm run build
```

On macOS, the current Tauri config can produce local app and DMG bundles:

```bash
npm run build:dmg
```

These local bundles are not beta release artifacts until signing,
notarization, checksums, and release manifest evidence exist.

## WSL Agent Instructions

Treat each WSL distro as a separate Linux device. Build and run DevRelay inside
the distro, keep `DEVRELAY_HOME` on the distro filesystem, and register a
separate workspace from any Windows-native checkout.

Recommended WSL layout:

```bash
export DEVRELAY_HOME="$HOME/.local/share/devrelay"
git clone <repo-url> "$HOME/src/devrelay"
cd "$HOME/src/devrelay"
cargo build --release -p devrelay-cli -p devrelay-agent
export PATH="$PWD/target/release:$PATH"
devrelay identity init
devrelay doctor wsl-filesystem --repo <project-path> --json
```

If the distro uses systemd user services:

```bash
devrelay agent install \
  --agent-bin "$PWD/target/release/devrelay-agent"
systemctl --user daemon-reload
systemctl --user enable --now devrelay-agent.service
```

If systemd user services are unavailable, run the agent in the foreground for
dogfood:

```bash
DEVRELAY_HOME="$HOME/.local/share/devrelay" \
  devrelay-agent --foreground --log-level info
```

Do not point Windows-native tools at a WSL-owned checkout through `\\wsl$`, and
do not point WSL tools at a Windows-owned checkout under `/mnt/c`. Use separate
clones and separate DevRelay device identities.

## Upgrade Path

Before replacing binaries:

```bash
devrelay doctor project-safety --repo <project-path> --manifest <manifest-path>
devrelay doctor environment --repo <project-path> --manifest <manifest-path>
devrelay checkpoint --repo <project-path> --manifest <manifest-path> --label "pre-upgrade"
```

For a dev-channel upgrade:

1. Stop the per-user agent with the platform command for your OS:
   `launchctl bootout gui/$(id -u)/com.devrelay.agent` on macOS or
   `systemctl --user disable --now devrelay-agent.service` on Linux.
2. Pull or check out the intended Git revision.
3. Rebuild `devrelay` and `devrelay-agent`.
4. Reinstall the agent service if the binary path changed.
5. Start the service and run `devrelay agent status --json`.
6. Re-run relevant doctors before handoff dogfood.

For beta, follow the release notes for the target version. If a metadata schema
migration is listed, keep the pre-migration backup path until rollback is no
longer needed. Downgrades across metadata migrations are unsupported unless the
backup from before the migration is restored.

## Uninstall Path

Remove the per-user agent service template and print the unload command:

```bash
devrelay agent uninstall
```

Run the printed platform command to unload the service:

- macOS: `launchctl bootout gui/$(id -u)/com.devrelay.agent`
- Linux: `systemctl --user disable --now devrelay-agent.service`

Then remove binaries or desktop bundles installed for the channel:

- Dev channel: remove copied binaries from your PATH, or delete the source
  checkout if it is no longer needed.
- macOS desktop dev bundle: remove the locally built `DevRelay.app` or DMG.
- Beta channel: use the package manager or installer removal path documented in
  that release's notes.

DevRelay state is user data. Remove it only after exporting diagnostics or
backing up anything you may need for recovery. The default state roots are:

- macOS: `~/Library/Application Support/DevRelay`
- Linux and WSL: `${XDG_DATA_HOME:-~/.local/share}/devrelay`
- Windows: `%LOCALAPPDATA%\DevRelay`

If `DEVRELAY_HOME` was set, that directory is the state root for that process
and service template.
