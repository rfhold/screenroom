# syntax=docker/dockerfile:1
FROM rust:1.98.1-trixie AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    libgtk-4-dev libadwaita-1-dev pkg-config jq libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /repo
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY proto ./proto
COPY apps/linux/Cargo.toml apps/linux/Cargo.lock ./apps/linux/
COPY apps/linux/src ./apps/linux/src
WORKDIR /repo/apps/linux
ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=2
RUN --mount=type=cache,id=screenroom-linux-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=screenroom-linux-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=screenroom-linux-target,target=/repo/apps/linux/target \
    cargo check --locked --bin screenroom-linux \
    && cargo test --locked --no-default-features --lib \
    && cargo test --locked --bin screenroom-linux \
    && cargo build --locked --features e2e --bin screenroom-linux \
    && cp target/debug/screenroom-linux /screenroom-linux
COPY apps/linux/tests/e2e.rs ./tests/e2e.rs
COPY apps/linux/tests/e2e/playback.rs ./tests/e2e/playback.rs
RUN --mount=type=cache,id=screenroom-linux-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=screenroom-linux-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=screenroom-linux-target,target=/repo/apps/linux/target \
    cargo test --locked --bin screenroom-linux \
    && cargo test --locked --features e2e --test e2e nondefault_origin_fault_tunnel -- --exact \
    && cargo test --locked --features e2e --test e2e playback::track_inventory_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::track_structure_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::track_eligibility_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::controls_trace_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::controls_future_layout_contract -- --exact --nocapture \
    && cargo test --locked --features e2e --test e2e playback::exact_search_request_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::exact_item_selector_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::stage_diagnostics_contract -- --exact \
    && cargo test --locked --features e2e --test e2e pointer_policy_contract -- --exact \
    && cargo test --locked --features e2e --test e2e pointer_snapshot_eligibility_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::title_only_control_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::readiness_diagnostic_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::readiness_shutdown_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::readiness_observation_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::settings_control_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::live_readiness_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::readiness_progress_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::modal_focus_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::modal_keyboard_policy_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::motion_policy_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::motion_diagnostic_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::idle_observation_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::readiness_stage_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::readiness_reveal_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::readiness_absence_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::readiness_ownership_trace_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::chrome_receipt_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::public_idle_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::idle_candidate_contract -- --exact \
    && cargo test --locked --features e2e --test e2e playback::idle_candidate_controls_contract -- --exact \
    && cargo test --locked --features e2e --test e2e --no-run --message-format=json-render-diagnostics > /build.json \
    && cp "$(jq -r 'select(.reason == "compiler-artifact" and .target.name == "e2e" and .profile.test) | .executable // empty' /build.json)" /e2e-test

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libgtk-4-1 libadwaita-1-0 mutter pipewire wireplumber dbus-daemon coreutils \
    gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-pipewire \
    gstreamer1.0-gtk4 gstreamer1.0-libav gstreamer1.0-plugins-bad gstreamer1.0-x gstreamer1.0-tools \
    at-spi2-core adwaita-icon-theme librsvg2-common fonts-dejavu-core curl socat python3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 e2e \
    && mkdir /artifacts /tmp/screenroom-runtime \
    && chmod 700 /tmp/screenroom-runtime \
    && chown e2e:e2e /artifacts /tmp/screenroom-runtime
COPY --from=builder /screenroom-linux /usr/local/bin/screenroom-linux
COPY --from=builder /e2e-test /usr/local/bin/e2e-test
COPY apps/linux/tests/e2e/container.sh /usr/local/bin/screenroom-e2e
COPY apps/linux/tests/e2e/origin.py /usr/local/bin/screenroom-origin.py
COPY apps/linux/tests/e2e/origin_test.py /usr/local/bin/screenroom-origin-test.py
RUN python3 /usr/local/bin/screenroom-origin-test.py /usr/local/bin/screenroom-origin.py
COPY --chmod=755 apps/linux/tests/e2e/app.sh /usr/local/bin/screenroom-linux-capture
COPY --chmod=755 apps/linux/scripts/check-playback-plugins.sh /usr/local/bin/screenroom-check-playback-plugins
USER e2e
WORKDIR /artifacts
ENV XDG_RUNTIME_DIR=/tmp/screenroom-runtime \
    LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
    GDK_BACKEND=wayland GSK_RENDERER=cairo GTK_A11Y=atspi \
    NO_AT_BRIDGE=0
ENTRYPOINT ["timeout", "--signal=TERM", "--kill-after=5s", "180s", "bash", "/usr/local/bin/screenroom-e2e"]
