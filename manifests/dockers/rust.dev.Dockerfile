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
# Uses the musl-cross toolchain (same family as the production build,
# manifests/dockers/rust.Dockerfile) but targets the LOCAL cluster arch
# (aarch64) so the binary actually runs in an arm64 kind cluster — prod targets
# x86_64. This is a build+run image (has a shell + cargo) — do NOT ship it.
ARG APP_NAME
ARG RUST_TARGET=aarch64-unknown-linux-musl

FROM messense/rust-musl-cross:aarch64-musl@sha256:ecae5dd62d1c938c14f8071d36c16fa699860aace03bfb5284fb1216474d2643 AS chef
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
# CARGO_PROFILE_DEV_DEBUG=1 keeps panic line info + usable backtraces but drops
# full DWARF, which is the largest slice of a debug target/ — smaller image to
# load into kind, and faster codegen on every in-container live_update rebuild.
ENV PORT=8080 \
    RUST_BACKTRACE=1 \
    CARGO_PROFILE_DEV_DEBUG=1

# Bake dependencies (dev profile). This is the expensive layer that live_update
# relies on staying warm in the running container.
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --locked --recipe-path recipe.json --target ${RUST_TARGET}

# First full build so target/ is warm; subsequent builds happen via live_update.
COPY Cargo.toml Cargo.lock ./
COPY apps/ apps/
COPY libs/ libs/
RUN cargo build --locked -p ${APP_NAME} --target ${RUST_TARGET} \
    && ln -sf /app/target/${RUST_TARGET}/debug/${APP_NAME} /usr/local/bin/app

EXPOSE ${PORT}
CMD ["app"]
