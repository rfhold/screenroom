FROM rust:1.98-trixie@sha256:a8a5f0a1e5fe7dfe1d352591e4a1c7dd2c08fd70475cae872cf3458ba0df0546 AS source
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
ENV CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0
FROM source AS build-catalog
RUN cargo build --locked -p screenroom-catalog
FROM source AS build-user
RUN cargo build --locked -p screenroom-user
FROM source AS build-worker
RUN cargo build --locked -p screenroom-index-worker
FROM source AS build-playback-gateway
RUN cargo build --locked -p screenroom-playback-gateway
FROM source AS build-index-operator
RUN cargo build --locked -p screenroom-index-operator
FROM source AS build-local
RUN cargo build --locked -p screenroom-catalog -p screenroom-user -p screenroom-index-worker -p screenroom-index-operator -p screenroom-playback-gateway

FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2 AS runtime-base
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
RUN groupadd --gid 10001 screenroom && useradd --uid 10001 --gid 10001 --no-create-home screenroom
USER 10001:10001
WORKDIR /app

FROM runtime-base AS service-runtime
ARG REVISION
LABEL org.opencontainers.image.revision=$REVISION

FROM service-runtime AS catalog
COPY --from=build-catalog /src/target/debug/screenroom-catalog /usr/local/bin/
FROM service-runtime AS user
COPY --from=build-user /src/target/debug/screenroom-user /src/target/debug/screenroom-user-init /usr/local/bin/
FROM service-runtime AS worker
COPY --from=build-worker /src/target/debug/screenroom-index-worker /usr/local/bin/
FROM service-runtime AS playback-gateway
COPY --from=build-playback-gateway /src/target/debug/screenroom-playback-gateway /usr/local/bin/
FROM service-runtime AS index-operator
COPY --from=build-index-operator /src/target/debug/screenroom-index-operator /usr/local/bin/

FROM runtime-base AS runtime
COPY --from=build-local /src/target/debug/screenroom-catalog /src/target/debug/screenroom-user /src/target/debug/screenroom-user-init /src/target/debug/screenroom-index-worker /src/target/debug/screenroom-index-operator /src/target/debug/screenroom-playback-gateway /usr/local/bin/
