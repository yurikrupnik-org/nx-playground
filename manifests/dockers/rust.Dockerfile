# syntax=docker/dockerfile:1
ARG APP_NAME
ARG RUST_TARGET=x86_64-unknown-linux-musl

# Single toolchain image for both planning and building. The musl-cross image
# cross-compiles to x86_64-unknown-linux-musl on any host arch, so local arm64
# builds pull the native arm64 variant (no QEMU). Pinned by digest for
# reproducibility; the readable tag is kept alongside for maintenance.
FROM messense/rust-musl-cross:x86_64-musl@sha256:ce75e9174325d4fbb3de85c309e2d7ca29f7500169bc4b5d2c611ff7e86d549a AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY apps/ apps/
COPY libs/ libs/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG APP_NAME
ARG RUST_TARGET

# Compile dependencies first; this layer is reused until the manifests/lockfile change.
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/app/target,id=rust-target,sharing=locked \
    cargo chef cook --release --locked --recipe-path recipe.json --target ${RUST_TARGET}

COPY Cargo.toml Cargo.lock ./
COPY apps/ apps/
COPY libs/ libs/

RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/app/target,id=rust-target,sharing=locked \
    cargo build --release --locked -p ${APP_NAME} --target ${RUST_TARGET} \
    && cp target/${RUST_TARGET}/release/${APP_NAME} /app-bin

FROM scratch AS rust
ARG APP_NAME

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /app-bin /app

# scratch has no /etc/passwd, so use a numeric UID:GID. This makes the image
# genuinely non-root and satisfies Kubernetes runAsNonRoot / restricted PSS.
USER 65534:65534

ENV PORT=8080 \
    RUST_BACKTRACE=1
EXPOSE ${PORT}

LABEL \
    org.opencontainers.image.title="${APP_NAME}" \
    org.opencontainers.image.source="playground" \
    org.opencontainers.image.description="Minimal Rust application from Nx monorepo" \
    security.non-root="true" \
    security.static-binary="true" \
    security.minimal-size="true" \
    security.no-shell="true" \
    security.distroless="false" \
    security.minimal="true" \
    security.base-image="scratch"

ENTRYPOINT ["/app"]
