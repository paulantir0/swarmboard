# swarmboard API: single static-ish binary + SQLite on a /data volume.
#   docker build -t swarmboard-api server/
#   docker run --rm -p 8191:8191 -v swarmboard-data:/data --env-file server/.env swarmboard-api

# ---- build ----------------------------------------------------------------------------------
FROM rust:1-bookworm AS builder
WORKDIR /app

# Dependency layer: build against a stub main so this layer is cached until Cargo.* change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src target/release/deps/swarmboard_server* target/release/swarmboard-server*

# App layer. migrations/ is embedded at compile time by sqlx::migrate!.
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked \
    && cp target/release/swarmboard-server /usr/local/bin/swarmboard-server

# ---- runtime --------------------------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --home-dir /data --shell /usr/sbin/nologin swarmboard \
    && mkdir -p /data \
    && chown swarmboard:swarmboard /data

COPY --from=builder /usr/local/bin/swarmboard-server /usr/local/bin/swarmboard-server

ENV BIND=0.0.0.0:8191 \
    DATA_DIR=/data \
    DATABASE_URL=sqlite:///data/swarmboard.db \
    RUST_LOG=info,tower_http=info,sqlx=warn

USER swarmboard
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8191

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8191/healthz || exit 1

CMD ["/usr/local/bin/swarmboard-server"]
