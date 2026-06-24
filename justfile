set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

preflight: fmt-check clippy test

audit:
    cargo audit --deny warnings

dependency-inventory:
    python3 scripts/dependency_inventory.py --out target/dependency-inventory

resource-benchmark:
    python3 scripts/resource_benchmark.py

tooling: audit dependency-inventory
