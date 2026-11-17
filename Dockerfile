# syntax=docker/dockerfile:1
FROM node:24-bookworm-slim AS web
WORKDIR /build/apps/web
COPY apps/web/package*.json ./
RUN npm ci
COPY apps/web/ ./
RUN npm run build

FROM rust:1.98-bookworm AS api
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY apps/api/ apps/api/
RUN cargo build --locked --release -p ferrix-api

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 ferrix && useradd --uid 10001 --gid ferrix --no-create-home ferrix
COPY --from=api /build/target/release/ferrix-api /usr/local/bin/ferrix-api
COPY --from=web /build/apps/web/dist /opt/ferrix/web
ENV FERRIX_BIND=0.0.0.0:8080 FERRIX_STATIC_DIR=/opt/ferrix/web RUST_LOG=ferrix_api=info,tower_http=info
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=3s --start-period=30s --retries=5 CMD curl -fsS http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["/usr/local/bin/ferrix-api"]
