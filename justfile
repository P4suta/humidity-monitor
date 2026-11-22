default:
    @just --list

check:
    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace
    cargo audit
    gitleaks git --redact --no-banner

mutation:
    docker build --target meter -f verification/Containerfile.mutations -t humidity-monitor-mutations .
    docker run --rm --network=none --cpus=2 --memory=2g --memory-swap=2g --pids-limit=256 "$(docker image inspect --format='{{ "{{.Id}}" }}' humidity-monitor-mutations)" timeout 900 cargo mutants --jobs 1 --timeout 20

storage-endurance:
    cargo test --locked --test storage_upgrade endurance:: -- --ignored --test-threads=1

storage-mutation file="src/store.rs" filter="." tests="":
    docker build --target storage -f verification/Containerfile.mutations -t humidity-storage-mutations .
    docker run --rm --network=none --cpus=2 --memory=2g --memory-swap=2g --pids-limit=256 "$(docker image inspect --format='{{ "{{.Id}}" }}' humidity-storage-mutations)" timeout 1200 cargo mutants --in-place --timeout 25 --file {{ quote(file) }} --re {{ quote(filter) }} --cargo-test-arg=--lib --cargo-test-arg=--test=storage --cargo-test-arg=--test=storage_upgrade {{ if tests == "" { "" } else { "--cargo-test-arg=" + quote(tests) } }}
