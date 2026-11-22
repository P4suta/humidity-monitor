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

receiver-footprint:
    @test "$(docker version --format='{{ "{{.Server.Arch}}" }}')" = arm64 || { echo 'The receiver memory check requires a native ARM64 Docker engine.' >&2; exit 1; }
    docker build --platform linux/arm64 --target footprint -f Containerfile -t humidity-receiver-footprint .
    docker run --rm --platform linux/arm64 --network=none --cpus=2 --memory=192m --memory-swap=192m --pids-limit=128 "$(docker image inspect --format='{{ "{{.Id}}" }}' humidity-receiver-footprint)"

storage-mutation file="src/store.rs" filter="." tests="":
    docker build --target storage -f verification/Containerfile.mutations -t humidity-storage-mutations .
    docker run --rm --network=none --cpus=2 --memory=2g --memory-swap=2g --pids-limit=256 --tmpfs /test-tmp:rw,nosuid,nodev,size=192m,mode=1777 --env TMPDIR=/test-tmp "$(docker image inspect --format='{{ "{{.Id}}" }}' humidity-storage-mutations)" timeout 1200 cargo mutants --in-place --timeout 60 --file {{ quote(file) }} --re {{ quote(filter) }} --cargo-test-arg=--lib --cargo-test-arg=--test=storage --cargo-test-arg=--test=storage_upgrade {{ if tests == "" { "" } else { "--cargo-test-arg=" + quote(tests) } }}
