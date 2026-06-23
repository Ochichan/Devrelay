#![no_main]

use devrelay_core::{StatusEntryKind, parse_status_porcelain_v2};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = std::str::from_utf8(data)
        && let Ok(status) = parse_status_porcelain_v2(raw)
    {
        let summary = status.summary();
        assert_eq!(summary.head_oid, status.head_oid);
        assert_eq!(summary.branch, status.branch);
        assert_eq!(summary.upstream, status.upstream);
        assert_eq!(summary.counts, status.counts);

        assert_eq!(
            status
                .entries
                .iter()
                .filter(|entry| entry.kind == StatusEntryKind::Untracked)
                .count(),
            status.counts.untracked
        );
        assert_eq!(
            status
                .entries
                .iter()
                .filter(|entry| entry.kind == StatusEntryKind::Ignored)
                .count(),
            status.counts.ignored
        );
        assert_eq!(
            status
                .entries
                .iter()
                .filter(|entry| entry.kind == StatusEntryKind::Unmerged)
                .count(),
            status.counts.unmerged
        );
        assert_eq!(
            status
                .entries
                .iter()
                .filter_map(|entry| entry.xy.as_deref())
                .filter(|xy| xy.chars().next().map(|value| value != '.').unwrap_or(false))
                .count(),
            status.counts.staged
        );
        assert_eq!(
            status
                .entries
                .iter()
                .filter_map(|entry| entry.xy.as_deref())
                .filter(|xy| xy.chars().nth(1).map(|value| value != '.').unwrap_or(false))
                .count(),
            status.counts.unstaged
        );
    }
});
