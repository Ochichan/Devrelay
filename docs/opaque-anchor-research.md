# Opaque Anchor Research

Last updated: 2026-06-24

This note evaluates whether the North Star release should include an anchor
mode where the anchor stores only opaque encrypted data and cannot inspect Git
objects, CAS chunks, or snapshot metadata.

## Decision

Do not include opaque anchor in the North Star release.

DevRelay should keep the North Star anchor focused on authenticated access,
namespace restrictions, revocation, integrity verification, retention, and
backup/restore. Opaque storage is valuable but cuts across Git negotiation,
deduplication, partial fetch, diagnostics, and recovery. It should remain a
post-North-Star research track unless a strong privacy requirement appears.

No proof of concept is justified before beta because the current blocker is
real-device handoff reliability, not anchor confidentiality from the user's own
trusted infrastructure.

## Encrypted Bundle Design

An encrypted bundle design would package a transfer unit as one encrypted blob:

- snapshot metadata
- Git pack or bundle for required refs
- CAS reachability roots and chunks
- sidecar manifests
- signed manifest of plaintext object IDs and state hashes

The source would create a random content key, encrypt the bundle, and seal the
key to each authorized target device or fabric key. The anchor would store the
ciphertext and serve it by opaque bundle ID.

Advantages:

- Simple anchor storage model.
- Anchor cannot read paths, metadata, object contents, or sidecar contents.
- Easy deletion and retention by bundle ID.

Costs:

- Poor incremental transfer; small changes can require large encrypted bundles.
- Anchor cannot answer Git negotiation or object-availability queries.
- CAS chunk deduplication is mostly lost.
- Recovery diagnostics become weaker because the anchor cannot inspect missing
  object or metadata shape.
- Multi-target access requires multiple wrapped keys or re-encryption.

## Encrypted Chunk Design

An encrypted chunk design would preserve chunk-level storage:

- Split Git packs/CAS payloads into chunks.
- Encrypt each chunk with a per-snapshot or per-object key.
- Store an encrypted manifest mapping logical object IDs to ciphertext chunks.
- Seal manifest keys to authorized target devices.

Two addressing modes are possible:

- **Plaintext-derived addressing:** deduplication works, but the anchor can see
  equality of content across projects/devices.
- **Ciphertext-derived addressing with random keys:** stronger privacy, but
  deduplication disappears and reachability roots become target/key specific.

Advantages:

- Better partial retrieval than encrypted bundles.
- Some retention and reachability concepts map to current CAS roots.
- Can be layered near existing CAS code.

Costs:

- Key management becomes part of every data-plane route.
- Chunk-level integrity must validate both ciphertext and plaintext hashes.
- Deduplication leaks content equality unless disabled.
- Anchor cannot safely garbage-collect by semantic snapshot state without
  trusted encrypted manifests.
- Re-keying for revocation or new targets is expensive.

## Deduplication Tradeoff

Current CAS deduplicates by content identity. Opaque anchor changes that:

- Plaintext-hash dedup leaks when two devices/projects store the same content.
- Convergent encryption also leaks equality and is risky for low-entropy files.
- Random-key encryption hides equality but prevents global deduplication.
- Per-project dedup is a middle ground but still leaks equality within that
  project's encrypted store.

For DevRelay's expected private-machine use case, predictable recovery and
simple integrity checks matter more than opaque-anchor deduplication research.

## Git Negotiation Tradeoff

The existing anchor can serve Git refs/packs and verify snapshot availability.
Opaque anchor blocks that model:

- The anchor cannot advertise object reachability without seeing Git object IDs
  or an encrypted manifest it can trust but not interpret.
- Targets cannot ask the anchor for normal Git negotiation unless the target
  already has and decrypts the relevant manifest.
- Partial clone and sparse checkout become harder because missing blob fetches
  need encrypted manifest lookup before object transfer.
- Diagnostics cannot distinguish "anchor lacks object" from "target lacks key"
  without a new error/reporting layer.

The alternative is a client-side bundle protocol: the source creates encrypted
transfer bundles and the target downloads/decrypts whole bundles. That is safer
for privacy but worse for DevRelay's low-friction continuation goal.

## North Star Position

Opaque anchor is excluded from the North Star release. The release should state:

- Anchor data is authenticated, namespace-limited, integrity-checked, audited,
  and revocation-aware.
- Anchor data is not promised to be opaque to the machine/account operating the
  anchor.
- Users who need storage opacity should use full-disk encryption or host the
  anchor only on devices they administratively trust.

Future work can revisit opaque anchor after:

- real-device handoff is reliable,
- backup anchor restore is implemented,
- signed release/provenance is complete,
- and a concrete privacy requirement justifies the complexity.
