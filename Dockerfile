# Multi-arch (amd64 + arm64) build via cross-compilation.
#
# We always run the Rust compiler on the native build host ($BUILDPLATFORM)
# and cross-compile to $TARGETPLATFORM. This is ~5-10x faster than running
# rustc under QEMU emulation and avoids emulator crashes when linking the
# aws-lc-sys C sources that rustls 0.23 pulls in by default.

FROM --platform=$BUILDPLATFORM rust:1.94-bookworm AS builder

ARG BUILDPLATFORM
ARG TARGETPLATFORM
ARG TARGETARCH

# cmake is needed by aws-lc-sys. Cross toolchains for both possible target
# architectures are installed so the image builds regardless of build host.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake \
        gcc-aarch64-linux-gnu g++-aarch64-linux-gnu libc6-dev-arm64-cross \
        gcc-x86-64-linux-gnu g++-x86-64-linux-gnu libc6-dev-amd64-cross \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    case "$TARGETARCH" in \
        amd64) RUST_TARGET=x86_64-unknown-linux-gnu ;; \
        arm64) RUST_TARGET=aarch64-unknown-linux-gnu ;; \
        *) echo "unsupported TARGETARCH=$TARGETARCH" >&2; exit 1 ;; \
    esac; \
    rustup target add "$RUST_TARGET"; \
    echo "$RUST_TARGET" > /tmp/rust_target

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CXX_x86_64_unknown_linux_gnu=x86_64-linux-gnu-g++ \
    AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar

WORKDIR /app

# Prime the dep cache with a stub main so the heavy dep compile is cached
# separately from app code changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
 && cargo build --release --target "$(cat /tmp/rust_target)" \
 && rm -rf src "target/$(cat /tmp/rust_target)/release/deps/chronicle_data_api"*

COPY src/ src/
COPY migrations/ migrations/
RUN cargo build --release --target "$(cat /tmp/rust_target)" \
 && mkdir -p /out \
 && cp "target/$(cat /tmp/rust_target)/release/chronicle-data-api" /out/chronicle-data-api

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out/chronicle-data-api /usr/local/bin/chronicle-data-api
COPY --from=builder /app/migrations /migrations

ENV RUST_LOG=info
EXPOSE 8001
CMD ["chronicle-data-api"]
