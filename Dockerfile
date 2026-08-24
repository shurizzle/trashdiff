# --- Stage 1: Builder ---
# Debian Slim base provides glibc natively
FROM rust:1-slim-bookworm AS builder

WORKDIR /app
COPY . .

RUN cargo build --release --locked

# --- Stage 2: Final image ---
# distroless cc-debian12 provides glibc, libgcc and root CA certificates
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /app/target/release/trashdiff /app_bin

ENTRYPOINT ["/app_bin"]
