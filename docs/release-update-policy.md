# Release, Update, Provenance, And Migration Policy

Last updated: 2026-06-24

This policy defines the release channel model, signing/provenance expectations,
schema support windows, and rollback behavior required before beta.

## Release Channels

DevRelay uses four channels:

- **dev:** local builds from a workspace, usually `cargo run` or `cargo test`.
  Dev builds may use unreleased schema versions and are not auto-updated.
- **nightly:** CI-built artifacts from `main` for internal dogfood. Nightly
  builds may contain experimental features but must never run destructive
  migrations without creating a local backup first.
- **beta:** signed artifacts for real daily use on private machines. Beta
  builds may include compatibility shims and must preserve supported metadata
  and snapshot schema windows.
- **stable:** future channel after North Star release candidate. Stable requires
  signed installers, published checksums, provenance attestations, rollback
  instructions, and completed security/release gates.

Channel rules:

- A workspace may move from dev/nightly to beta only after `devrelay doctor`
  reports no migration blockers.
- Automatic updates are disabled for dev builds.
- Beta and stable updates must show release notes when schema migrations,
  command trust behavior, or transport trust behavior changes.
- A downgrade across a metadata schema migration is unsupported unless the user
  restores the backup made before the migration.

## Signed Release Strategy

Required signing by platform:

- **macOS:** sign the agent, CLI, desktop app, helper binaries, and DMG with a
  Developer ID certificate; notarize distributable artifacts; verify with
  `codesign --verify --deep --strict` and `spctl`.
- **Linux:** publish detached signatures and checksums for tarballs/packages;
  prefer Sigstore or minisign for beta, and document the verification command.
- **Windows:** use Authenticode for installer and binaries before any Windows UI
  claim; Windows signing may lag macOS/Linux dogfood but must be complete before
  broad Windows support.

Signing rules:

- Every release artifact has a SHA-256 checksum.
- The release manifest lists artifact filename, platform key, version, Git
  commit, build profile, checksum, signature, and schema versions.
- Signing keys are not stored in the repository or CI logs.
- Failed signature or notarization verification blocks beta/stable promotion.

## Binary Provenance Strategy

Each beta/stable release should publish provenance metadata with:

- Git commit and tag.
- Cargo.lock hash and Rust toolchain version.
- build target triple and platform key.
- build profile and enabled feature flags.
- generated artifact checksums.
- schema versions supported by the binary.
- CI workflow/run identity or local release operator identity.

Before stable, add an SBOM for bundled binaries and third-party dependencies.
Reproducibility is best-effort for beta and should be tightened before stable.

## Metadata Schema Support Window

Metadata DB migrations are forward-only. See `docs/migration-rollback-policy.md`.

Support window:

- Beta/stable binaries must migrate metadata DBs created by any beta/stable
  release from the previous 12 months or previous two minor versions, whichever
  is longer.
- Dev/nightly schema versions are not guaranteed outside the current branch, but
  migrations must remain transactional.
- A destructive migration requires a pre-migration backup, release-note callout,
  and manual recovery instructions.
- The binary must refuse to start if the metadata DB schema is newer than the
  binary supports.

## Snapshot Schema Support Window

Snapshot metadata is longer-lived than local metadata. Support window:

- Beta/stable binaries must read all snapshot schema versions emitted by
  supported beta/stable releases.
- Writers may emit only the current snapshot schema version.
- Readers must preserve unknown forward-compatible fields when practical and
  must fail closed if a required field or object ID cannot be validated.
- Fixtures for every supported snapshot schema version must remain in tests.
- Dropping support for a snapshot schema version requires a migration/export
  path and release-note callout.

## Rollback Expectations

Rollback means returning to a previous binary and restoring compatible state. It
does not mean running down migrations.

Required behavior:

- Before metadata migrations, create a local backup copy of the metadata DB and
  record its path in logs/diagnostics.
- If an update fails before migration starts, the previous binary can continue
  using the existing DB.
- If an update fails after migration, the rollback path is to restore the backup
  DB and then launch the previous binary.
- Snapshot Git refs, CAS chunks, pinned snapshots, latest canonical snapshots,
  and recovery-critical data must not be pruned as part of binary rollback.
- Release notes must state whether rollback is automatic, manual backup restore,
  or unsupported for that release.

## Release Gate Evidence

Before beta/stable promotion, collect:

- `cargo test` and UI/package checks for the target platforms.
- signature/notarization verification outputs.
- checksums and release manifest.
- migration fixture test results.
- manual rollback rehearsal notes for at least macOS and Linux.
- known issues and unsupported downgrade list.
