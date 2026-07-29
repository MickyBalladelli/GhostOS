# SynOS bootable Docker image
# Multi-stage build: first stage builds SynOS, second stage provides the QEMU runtime.

# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.88-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    lld \
    dosfstools \
    mtools \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rust-src llvm-tools \
    && rustup target add x86_64-unknown-none x86_64-unknown-uefi \
    && cargo install cargo-uefi --version 0.3.0

WORKDIR /synos
COPY . .

# Build both BIOS and UEFI images.
RUN ./scripts/build-bios-image.sh
RUN cargo uefi --release && ./scripts/build-portable-image.sh

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    qemu-system-x86 \
    # QEMU CXL and ivshmem devices are packaged with qemu-system-x86 on bookworm.
    procps \
    ripgrep \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /synos

# Copy built images from the builder stage.
COPY --from=builder /synos/build/bios/synos-bios.img ./build/bios/
COPY --from=builder /synos/build/portable/synos.img ./build/portable/

# Copy the QEMU cluster launcher and helpers.
COPY scripts/qemu-cluster.sh ./scripts/
COPY scripts/qemu-cluster-fail-node.sh ./scripts/
COPY scripts/docker-entrypoint.sh /usr/local/bin/

# Expose VNC ports for up to 8 cluster nodes.
EXPOSE 5900-5907

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]