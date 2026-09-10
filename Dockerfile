# syntax=docker/dockerfile:1

FROM node:20-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
ARG NEXT_PUBLIC_BOT_USERNAME
RUN test -n "$NEXT_PUBLIC_BOT_USERNAME"
ENV NEXT_PUBLIC_BOT_USERNAME=$NEXT_PUBLIC_BOT_USERNAME
RUN npm run build

FROM rust:1.96-bookworm AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build -p pustol-api --release --bin pustol-api \
    && cargo build -p pustol-api --release --bin seed \
    && mkdir -p /out \
    && cp target/release/pustol-api target/release/seed /out/

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust /out/pustol-api /usr/local/bin/pustol-api
COPY --from=rust /out/seed /usr/local/bin/seed
COPY --from=web /src/web/out /var/lib/pustol/web
ENV PUSTOL_ASSETS_DIR=/var/lib/pustol/web
STOPSIGNAL SIGINT
CMD ["pustol-api"]
