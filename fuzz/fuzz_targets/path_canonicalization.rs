#![no_main]

use devrelay_core::normalize_workspace_relative_path;
use libfuzzer_sys::fuzz_target;
use std::path::{Component, Path};

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = std::str::from_utf8(data)
        && let Some(normalized) = normalize_workspace_relative_path(Path::new(raw))
    {
        assert!(!normalized.is_absolute());
        assert!(
            normalized
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        );
    }
});
