FROM node:24-bookworm-slim AS node
FROM rust:1.98-trixie@sha256:a8a5f0a1e5fe7dfe1d352591e4a1c7dd2c08fd70475cae872cf3458ba0df0546 AS build
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules/npm /opt/npm
ENV PATH="/opt/pnpm/bin:${PATH}" SCREENROOM_CORE_PROFILE=release VITE_SCREENROOM_SAME_ORIGIN=1
RUN node /opt/npm/bin/npm-cli.js install --prefix /opt/pnpm --global pnpm@12.18.0
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
RUN rustup target add wasm32-unknown-unknown \
    && cd / \
    && cargo install --locked wasm-bindgen-cli --version "$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/"/,"",$3); print $3}' /src/Cargo.lock)"
COPY crates ./crates
COPY scripts/build-client-core.sh ./scripts/
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY apps/web/package.json ./apps/web/package.json
COPY website/package.json ./website/package.json
RUN pnpm install --frozen-lockfile --ignore-scripts --filter @screenroom/web
COPY apps/web ./apps/web
RUN pnpm --filter @screenroom/web build

FROM nginx:1.28-alpine AS web
ARG REVISION=local
LABEL org.opencontainers.image.revision=$REVISION
COPY apps/web/nginx.conf /etc/nginx/nginx.conf
COPY --from=build /src/apps/web/build /usr/share/nginx/html
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["nginx", "-g", "daemon off;"]
