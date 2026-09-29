# Keep the bun version in sync with `packageManager` in web/package.json.
FROM --platform=$BUILDPLATFORM oven/bun:1.4.2-alpine AS frontend
WORKDIR /src/web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web .
RUN bun run build

FROM --platform=$BUILDPLATFORM rust:1.98-bookworm AS chef
WORKDIR /app
# Install the toolchain pinned in rust-toolchain.toml up front: every later
# cargo and rustup command in /app resolves to it.
COPY rust-toolchain.toml .
RUN rustup toolchain install
RUN cargo install cargo-chef --version 0.1.78 --locked

FROM --platform=$BUILDPLATFORM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM --platform=$BUILDPLATFORM chef AS builder

ARG TARGETPLATFORM
RUN case "${TARGETPLATFORM}" in \
      "linux/arm64") echo "aarch64-unknown-linux-gnu" > /target.txt && echo "-C linker=aarch64-linux-gnu-gcc" > /flags.txt ;; \
      "linux/amd64") echo "x86_64-unknown-linux-gnu" > /target.txt && echo "-C linker=x86_64-linux-gnu-gcc" > /flags.txt ;; \
      *) echo "Unsupported platform: ${TARGETPLATFORM}" >&2 && exit 1 ;; \
    esac
RUN export DEBIAN_FRONTEND=noninteractive && \
    apt-get update && \
    apt-get install -yq build-essential g++-aarch64-linux-gnu binutils-aarch64-linux-gnu && \
    rm -rf /var/lib/apt/lists/*
RUN rustup target add "$(cat /target.txt)"

COPY --from=planner /app/recipe.json recipe.json
RUN RUSTFLAGS="$(cat /flags.txt)" cargo chef cook --profile dist --target "$(cat /target.txt)" --features embed-frontend --recipe-path recipe.json
COPY . .
COPY --from=frontend /src/web/dist web/dist
RUN RUSTFLAGS="$(cat /flags.txt)" cargo build --profile dist --target "$(cat /target.txt)" --features embed-frontend
RUN mv "./target/$(cat /target.txt)/dist/rustlog" /rustlog

FROM debian:bookworm-slim AS runtime
# reqwest verifies TLS certificates against the system store.
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*
RUN useradd rustlog && mkdir /logs && mkdir /app && chown rustlog: /logs /app
COPY --from=builder /rustlog /usr/local/bin/rustlog
WORKDIR /app
USER rustlog
EXPOSE 8026
CMD ["/usr/local/bin/rustlog"]
