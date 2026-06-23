#![no_main]

use devrelay_core::CasManifest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = std::str::from_utf8(data)
        && let Ok(manifest) = serde_json::from_str::<CasManifest>(raw)
    {
        let _ = manifest.validate();
    }
});
