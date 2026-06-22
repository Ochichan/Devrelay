# Release Notes Process

DevRelay does not have public releases yet. Until the first release candidate,
use this file as the release note process.

## Before A Release

1. Run `just preflight` locally.
2. Confirm CI is green on macOS, Linux, and Windows.
3. Confirm the relevant milestone exit gate in `docs/north-star-checklist.md`.
4. Summarize user-visible changes, data safety behavior, and known limitations.
5. Copy the final summary into `CHANGELOG.md` and the release artifact.

## Required Sections

- Highlights
- Compatibility
- Data safety notes
- Recovery notes
- Known limitations

