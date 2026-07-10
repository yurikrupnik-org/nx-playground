# syntax=docker/dockerfile:1
#
# LOCAL-DEVELOPMENT image for Tilt. Optimized for FAST iteration, not image size.
# Pair it with Tilt live_update (see manifests/dockers/README or the app Tiltfile):
#   * dependencies are compiled ONCE (cargo-chef) and baked into the image;
#   * the cargo toolchain + a warm target/ stay INSIDE the container, so
#     live_update recompiles only the changed workspace crates — deps are never
#     rebuilt and the image is never rebuilt on a code change;
#   * dev profile (opt-level=0, incremental) — no release LTO.
#
# Uses the SAME toolchain image as the production build
# (manifests/dockers/rust.Dockerfile), so every crate/C-dep that compiles in prod
# compiles here too. This is a build+run image (has a shell + cargo) — do NOT ship it.
ARG APP_NAME
ARG RUST_TARGET=x86_64-unknown-linux-musl

FROM messense/rust-musl-cross:x86_64-musl@sha256:ce75e9174325d4fbb3de85c309e2d7ca29f7500169bc4b5d2c611ff7e86d549a AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY apps/ apps/
COPY libs/ libs/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS dev
ARG APP_NAME
ARG RUST_TARGET
ENV PORT=8080 \
    RUST_BACKTRACE=1

# Bake dependencies (dev profile). This is the expensive layer that live_update
# relies on staying warm in the running container.
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/root/.cargo/registry \
    cargo chef cook --locked --recipe-path recipe.json --target ${RUST_TARGET}

# First full build so target/ is warm; subsequent builds happen via live_update.
COPY Cargo.toml Cargo.lock ./
COPY apps/ apps/
COPY libs/ libs/
RUN --mount=type=cache,target=/root/.cargo/registry \
    cargo build --locked -p ${APP_NAME} --target ${RUST_TARGET} \
    && ln -sf /app/target/${RUST_TARGET}/debug/${APP_NAME} /usr/local/bin/app

EXPOSE ${PORT}
CMD ["app"]
