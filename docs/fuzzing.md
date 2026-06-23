# Fuzzing

Last updated: 2026-06-23

DevRelay fuzzing covers parsers and trust-boundary payloads where malformed
input should never panic or violate parser invariants.

## Targets

The fuzz crate lives in [`fuzz/`](../fuzz) and currently defines these targets:

- `manifest_parser`: parses `devrelay.toml` and computes the execution trust
  hash for valid manifests.
- `porcelain_parser`: parses Git porcelain v2 status records and checks summary
  count invariants.
- `path_canonicalization`: normalizes workspace-relative paths and checks that
  accepted paths cannot be absolute or contain traversal components.
- `cas_manifest`: deserializes and validates CAS manifests.
- `network_api_payload`: parses local JSON-RPC requests and deserializes known
  method params. This target does not cover the unimplemented M4.5 remote
  Control API.

Each target has seed inputs under `fuzz/corpus/<target>/`.

## Prerequisites

Install a nightly toolchain:

```bash
rustup toolchain install nightly
```

For full fuzzing, install `cargo-fuzz`:

```bash
cargo install cargo-fuzz
```

If `cargo-fuzz` is unavailable, use the smoke commands below to compile and run
the harnesses directly.

## Smoke Check

Run a target against its committed corpus without requiring `cargo-fuzz`:

```bash
cargo +nightly run --manifest-path fuzz/Cargo.toml --bin manifest_parser -- -runs=1 fuzz/corpus/manifest_parser
cargo +nightly run --manifest-path fuzz/Cargo.toml --bin porcelain_parser -- -runs=1 fuzz/corpus/porcelain_parser
cargo +nightly run --manifest-path fuzz/Cargo.toml --bin path_canonicalization -- -runs=1 fuzz/corpus/path_canonicalization
cargo +nightly run --manifest-path fuzz/Cargo.toml --bin cas_manifest -- -runs=1 fuzz/corpus/cas_manifest
cargo +nightly run --manifest-path fuzz/Cargo.toml --bin network_api_payload -- -runs=1 fuzz/corpus/network_api_payload
```

Direct harness runs on macOS may print sanitizer symbol warnings when they are
not launched through `cargo-fuzz`; treat exit status as the smoke-check result.

## Fuzz Run

Run one target for five minutes:

```bash
cargo +nightly fuzz run manifest_parser fuzz/corpus/manifest_parser -- -max_total_time=300
```

Replace `manifest_parser` with another target name and pass the matching corpus
directory. Longer unattended runs should use `-max_total_time` or `-runs` so
they terminate predictably in automation.

## Nightly CI

`.github/workflows/fuzz.yml` runs on a daily schedule and by manual dispatch.
It first compiles every harness and runs the committed corpus once, then runs
each target through `cargo-fuzz` for 60 seconds. Failed runs upload
`fuzz/artifacts/<target>/` for local reproduction.

## Crash Reproduction

When `cargo-fuzz` finds a crash, it writes an artifact under
`fuzz/artifacts/<target>/`. Reproduce with:

```bash
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<artifact>
```

Commit new minimized regression inputs to `fuzz/corpus/<target>/` only after the
underlying bug is fixed or the input is intentionally accepted as a permanent
regression seed.
