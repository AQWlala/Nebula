# syntax=docker/dockerfile:1.6
# T-D-C-05: 多架构支持（linux/amd64 + linux/arm64）
#
# 多架构构建（需 Docker buildx + QEMU）:
#   docker buildx create --use --name multiarch
#   docker buildx build --platform linux/amd64,linux/arm64 \
#       -t nebula:latest --push .
#
# 单架构构建:
#   docker build -t nebula .
#
# 注意: arm64 构建通过 QEMU 用户态模拟原生编译,速度较慢但可靠
# （reqwest 默认 native-tls 依赖 OpenSSL,交叉编译需安装 arm64
# 版本 libssl-dev,QEMU 方案避免此复杂性）。

# Stage 1: Build
FROM rust:1-bookworm AS builder
WORKDIR /app
COPY src-tauri/ ./src-tauri/
COPY src/ ./src/
COPY package.json ./
RUN sed -i 's/\r$//' ./src-tauri/entrypoint.sh
RUN cd src-tauri && CARGO_INCREMENTAL=0 cargo build --release --features headless --no-default-features

# Stage 2: Runtime
FROM debian:bookworm-slim

# T-D-C-05: 安装 ca-certificates（TLS）和 curl（HEALTHCHECK 探针）
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# T-D-C-05: 非 root 用户运行（容器安全最佳实践）
# 使用固定 UID/GID 1001,避免与宿主机常见 UID 冲突
RUN groupadd --system --gid 1001 nebula \
    && useradd --system --uid 1001 --gid nebula \
       --home-dir /data --shell /sbin/nologin nebula

COPY --from=builder /app/src-tauri/target/release/nebula /usr/local/bin/nebula
COPY src-tauri/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# 创建数据目录并设置权限（必须在 VOLUME 声明前 chown,
# 否则 named volume 会以 root 权限挂载,nebula 用户无法写入）
RUN mkdir -p /data /keychain /logs \
    && chown -R nebula:nebula /data /keychain /logs \
              /usr/local/bin/nebula /entrypoint.sh

USER nebula

VOLUME ["/data", "/keychain", "/logs"]
EXPOSE 50051 8080

ENV NEBULA_DB=/data/nebula.db
ENV NEBULA_LANCE=/data/nebula_lance
ENV NEBULA_LOG_DIR=/logs
ENV NEBULA_KEYCHAIN_DIR=/keychain
ENV RUST_LOG=info

# T-D-C-05: HEALTHCHECK — 探测 REST API /api/health 端点
# 容器内必须 bind 0.0.0.0（设置 NEBULA_REST_ADDR=0.0.0.0:8080）
# 否则 127.0.0.1 bind 在容器内可访问但端口映射无效
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1

ENTRYPOINT ["/entrypoint.sh"]
CMD ["nebula"]
