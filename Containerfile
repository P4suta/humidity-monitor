FROM docker.io/library/rust:1.100.0-trixie@sha256:223cc32ddca8f3ee90c436dbc94b6b5f052ea7f9653c5201e9612f08ad7d21b8 AS build
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY crates/switchbot-meter/Cargo.toml crates/switchbot-meter/Cargo.toml
COPY crates/switchbot-meter/src crates/switchbot-meter/src
COPY src src
RUN cargo build --locked --release --bin humidity-monitor

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:cb87acc574abe719b6e5c547384defa2f7b1f24af9605d3f618cf852495d8be2
COPY --from=build /source/target/release/humidity-monitor /usr/local/bin/humidity-monitor
ENTRYPOINT ["/usr/local/bin/humidity-monitor"]
