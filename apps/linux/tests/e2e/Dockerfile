# syntax=docker/dockerfile:1
FROM rust:1.98.1-trixie AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    libgtk-4-dev libadwaita-1-dev pkg-config jq libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /repo
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY fixtures/catalog ./fixtures/catalog
COPY apps/linux/Cargo.toml apps/linux/Cargo.lock ./apps/linux/
COPY apps/linux/src ./apps/linux/src
WORKDIR /repo/apps/linux
ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=2
RUN --mount=type=cache,id=screenroom-linux-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=screenroom-linux-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=screenroom-linux-target,target=/repo/apps/linux/target \
    cargo check --locked --bin screenroom-linux \
    && cargo test --locked --bin screenroom-linux \
    && cargo build --locked --bin screenroom-linux \
    && cp target/debug/screenroom-linux /screenroom-linux
COPY apps/linux/tests/e2e.rs ./tests/e2e.rs
RUN --mount=type=cache,id=screenroom-linux-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=screenroom-linux-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=screenroom-linux-target,target=/repo/apps/linux/target \
    cargo test --locked --bin screenroom-linux \
    && cargo test --locked --features e2e --test e2e --no-run --message-format=json-render-diagnostics > /build.json \
    && cp "$(jq -r 'select(.reason == "compiler-artifact" and .target.name == "e2e" and .profile.test) | .executable // empty' /build.json)" /e2e-test
WORKDIR /repo
RUN --mount=type=cache,id=screenroom-linux-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=screenroom-linux-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=screenroom-linux-core-target,target=/repo/target \
    cargo build --locked -p screenroom-mock-server \
    && cp target/debug/screenroom-mock-server /screenroom-mock-server

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libgtk-4-1 libadwaita-1-0 mutter pipewire wireplumber dbus-daemon coreutils \
    gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-pipewire \
    at-spi2-core adwaita-icon-theme librsvg2-common fonts-dejavu-core curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 e2e \
    && mkdir /artifacts /tmp/screenroom-runtime \
    && chmod 700 /tmp/screenroom-runtime \
    && chown e2e:e2e /artifacts /tmp/screenroom-runtime
COPY --from=builder /screenroom-linux /usr/local/bin/screenroom-linux
COPY --from=builder /e2e-test /usr/local/bin/e2e-test
COPY --from=builder /screenroom-mock-server /usr/local/bin/screenroom-mock-server
COPY apps/linux/tests/e2e/container.sh /usr/local/bin/screenroom-e2e
COPY --chmod=755 apps/linux/tests/e2e/app.sh /usr/local/bin/screenroom-linux-capture
USER e2e
WORKDIR /artifacts
ENV XDG_RUNTIME_DIR=/tmp/screenroom-runtime \
    LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
    GDK_BACKEND=wayland GSK_RENDERER=cairo GTK_A11Y=atspi \
    NO_AT_BRIDGE=0
ENTRYPOINT ["timeout", "--signal=TERM", "--kill-after=5s", "75s", "bash", "/usr/local/bin/screenroom-e2e"]
