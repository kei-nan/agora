# syntax=docker/dockerfile:1
#
# Multi-arch (linux/amd64, linux/arm64) image for the OPRF committee-node orchestrator
# (changelog #082 — laptop/Raspberry Pi committee members, founding phase). See README.md
# before running this anywhere real: several pieces are provisional/stubbed pending two
# other in-progress efforts (the real pallet-identity storage/extrinsics, the real
# Wasm-compiled crypto core).
#
# ## Building for both architectures
#
# Built with `docker buildx` so each target architecture compiles NATIVELY under QEMU
# emulation, rather than cross-compiling from the host:
#
#   docker buildx create --use   # once, if you don't already have a buildx builder
#   docker buildx build --platform linux/amd64,linux/arm64 -t committee-node:dev .
#
# On a Linux host, if `linux/arm64` isn't already registered:
#   docker run --privileged --rm tonistiigi/binfmt --install arm64
#
# Why emulation instead of cross-compiling: this binary's dependency tree includes wasmtime
# (a JIT compiler with real per-target-arch codegen backends) and sp-core (schnorrkel /
# curve25519-dalek / several C-linked crypto primitives via the `full_crypto` feature) — both
# do build for aarch64-unknown-linux-gnu (both are used in real ARM-hosted Substrate
# deployments, and wasmtime lists Linux aarch64 as a Tier 1 supported target), but getting a
# correct aarch64 cross-linker/sysroot on the build host is its own yak to shave. QEMU
# emulation sidesteps that at the cost of slower non-native builds — the right tradeoff for a
# minimal skeleton; revisit if CI build time becomes a real constraint.
#
# NOTE ON VERIFICATION: this Dockerfile was authored and the underlying Rust crate compiles
# cleanly with `cargo build` on this development machine (see README.md's "Build status"), but
# no `docker buildx build` was actually run against it — no Docker daemon was reachable in the
# sandbox this was written in. Read the crate's own `cargo build` success as "the code is
# correct"; read this Dockerfile as "reasoned through, not empirically verified end-to-end."

FROM rust:slim-bookworm AS builder
WORKDIR /build

# `clang`: matches this repo's own root `.cargo/config.toml` linker choice for consistency;
# stock `rust:slim` images already ship a working default linker (`cc`/`gcc`) so this isn't
# strictly required, but keeping the same toolchain choice as the rest of the project avoids
# a second, differently-configured Rust build environment to reason about.
RUN apt-get update && apt-get install -y --no-install-recommends clang \
    && rm -rf /var/lib/apt/lists/*

# Cargo.toml/Cargo.lock copied first so dependency compilation is cached across rebuilds that
# only touch src/.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

# --- Runtime stage: small, no Rust toolchain, no build tooling ---
FROM debian:bookworm-slim AS runtime

# ca-certificates: reqwest/rustls needs a trust store if NODE_RPC_URL is ever an https(s)
# endpoint; harmless no-op for a plain http:// dev-chain endpoint.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/committee --shell /usr/sbin/nologin committee \
    && mkdir -p /keys /wasm && chown committee:committee /keys /wasm

COPY --from=builder /build/target/release/committee-node /usr/local/bin/committee-node

# /keys: the age-encrypted secrets file — see keys/README.md, this is an explicit
#   placeholder, NOT tamper-resistant storage.
# /wasm: the compiled OPRF crypto-core module — see wasm/README.md. If empty, the node runs
#   in STUB mode (obviously-fake evaluation output, never auto-submitted — see
#   ALLOW_STUB_SUBMISSION in README.md).
VOLUME ["/keys", "/wasm"]

USER committee
ENV NODE_RPC_URL=http://127.0.0.1:9944 \
    KEYS_FILE=/keys/committee-secrets.age \
    WASM_MODULE_PATH=/wasm/oprf-crypto-core.wasm \
    POLL_INTERVAL_SECS=30 \
    RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/committee-node"]
