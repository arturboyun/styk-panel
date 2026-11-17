# syntax=docker/dockerfile:1.7
# Build from the repository root: docker build -f deploy/Dockerfile .

FROM node:22-alpine AS web
WORKDIR /web
COPY frontend/package.json frontend/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY frontend/ ./
RUN NODE_ENV=production npm run build

FROM rust:1-alpine AS rust
# Cargo reads it from the environment; local builds pass a limit to keep the machine usable.
ARG CARGO_BUILD_JOBS
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p styk-panel -p styk-agent \
    && mkdir /out && cp target/release/styk-panel target/release/styk-agent /out/

# `docker build --target agent --output type=local,dest=dist .` extracts the host agent binary.
FROM scratch AS agent
COPY --from=rust /out/styk-agent /styk-agent

FROM gcr.io/distroless/static-debian12:nonroot AS runtime
ENV STYK_STATIC_DIR=/srv/www STYK_LISTEN=0.0.0.0:8080
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=5s --start-period=10s CMD ["styk-panel", "healthcheck"]
ENTRYPOINT ["styk-panel"]
CMD ["serve"]

# CI: binaries and static files are built on the runners (with caches) and passed in a
# small context: `<arch>/styk-panel` (musl binary, arch = amd64 | arm64) and `www/`.
FROM runtime AS prebuilt
ARG TARGETARCH
COPY ${TARGETARCH}/styk-panel /usr/local/bin/styk-panel
COPY www /srv/www

# Default target: everything built inside Docker.
FROM runtime
COPY --from=rust /out/styk-panel /usr/local/bin/styk-panel
COPY --from=web /web/out /srv/www
