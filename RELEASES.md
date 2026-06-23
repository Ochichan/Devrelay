# Release Notes Process

DevRelay does not have public releases yet. Until the first release candidate,
use this file as the release note process.

## Before A Release

1. Run `just preflight` locally.
2. Confirm CI is green on macOS, Linux, and Windows.
3. Confirm the relevant milestone exit gate in `docs/north-star-checklist.md`.
4. Confirm API surfaces changed by the release are documented in
   `docs/api-surface.md`.
5. Confirm data-loss invariants have named test evidence in
   `docs/data-loss-safety.md`.
6. Confirm resource claims have evidence from `docs/resource-benchmark.md`.
7. Summarize user-visible changes, data safety behavior, and known limitations.
8. Copy the final summary into `CHANGELOG.md` and the release artifact.

## Required Sections

- Highlights
- Compatibility
- Data safety notes
- Recovery notes
- Resource and battery notes
- Security boundary notes
- Known limitations
