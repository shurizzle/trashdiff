# --- Stage 0: chef ---
# cargo-chef lets us compile dependencies in a layer that only depends on the
# manifests, so editing sources recompiles this crate alone instead of the tree.
FROM rust:1-slim-bookworm AS chef

RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /app

# --- Stage 1: recipe ---
FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Stage 2: Builder ---
# Debian Slim base provides glibc natively
FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json

COPY . .
RUN cargo build --release --locked

# --- Stage 3: Final image ---
# distroless cc-debian12 provides glibc, libgcc and root CA certificates
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /app/target/release/trashdiff /app_bin

ENTRYPOINT ["/app_bin"]
