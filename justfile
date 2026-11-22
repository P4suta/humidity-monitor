default:
    @just --list

check:
    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace
    cargo audit
    gitleaks git --redact --no-banner

mutation:
    docker build -f verification/Containerfile.mutations -t humidity-monitor-mutations .
    docker run --rm --network=none --cpus=2 --memory=2g --memory-swap=2g --pids-limit=256 "$(docker image inspect --format='{{ "{{.Id}}" }}' humidity-monitor-mutations)" timeout 900 cargo mutants --jobs 1 --timeout 20
