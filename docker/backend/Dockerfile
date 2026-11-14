FROM rust:1.98-trixie@sha256:a8a5f0a1e5fe7dfe1d352591e4a1c7dd2c08fd70475cae872cf3458ba0df0546 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p screenroom-catalog -p screenroom-user -p screenroom-index-worker -p screenroom-index-operator
FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/debug/screenroom-catalog /src/target/debug/screenroom-user /src/target/debug/screenroom-user-init /src/target/debug/screenroom-index-worker /src/target/debug/screenroom-index-operator /usr/local/bin/
RUN groupadd --gid 10001 screenroom && useradd --uid 10001 --gid 10001 --no-create-home screenroom
USER 10001:10001
WORKDIR /app
