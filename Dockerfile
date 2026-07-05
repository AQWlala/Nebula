# Stage 1: Build
FROM rust:1.77-bookworm AS builder
WORKDIR /app
COPY src-tauri/ ./src-tauri/
COPY src/ ./src/
COPY package.json ./
RUN sed -i 's/\r$//' ./src-tauri/entrypoint.sh
RUN cd src-tauri && CARGO_INCREMENTAL=0 cargo build --release --features headless --no-default-features

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/src-tauri/target/release/nebula /usr/local/bin/
COPY src-tauri/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

VOLUME ["/data", "/keychain", "/logs"]
EXPOSE 50051 8080

ENV NEBULA_DB=/data/nebula.db
ENV NEBULA_LANCE=/data/nebula_lance
ENV NEBULA_LOG_DIR=/logs
ENV NEBULA_KEYCHAIN_DIR=/keychain
ENV RUST_LOG=info

ENTRYPOINT ["/entrypoint.sh"]
CMD ["nebula"]