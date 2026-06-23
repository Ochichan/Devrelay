#![no_main]

use devrelay_core::Manifest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = std::str::from_utf8(data)
        && let Ok(manifest) = Manifest::parse(raw)
    {
        let _ = manifest.execution_trust_hash();
    }
});
