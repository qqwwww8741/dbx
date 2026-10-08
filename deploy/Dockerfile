# syntax=docker/dockerfile:1
FROM node:22-bookworm-slim AS frontend
WORKDIR /app
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages/ packages/
RUN --mount=type=cache,target=/root/.local/share/pnpm/store pnpm install --frozen-lockfile
COPY apps/desktop/ apps/desktop/
COPY scripts/ scripts/
COPY plugins/ plugins/
COPY crates/dbx-core/assets/ crates/dbx-core/assets/
RUN pnpm build

FROM rust:1-bookworm AS backend
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends cmake pkg-config perl libfontconfig-dev libfreetype-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/usr/local/cargo/git cargo build --locked --release -p dbx-web

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates fontconfig fonts-dejavu-core libfreetype6 libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/dbx-web /usr/local/bin/dbx-web
COPY --from=frontend /app/dist /app/static
ENV DBX_STATIC_DIR=/app/static DBX_DATA_DIR=/app/data DBX_BACKUP_ROOT=/app/backups
EXPOSE 4224
CMD ["dbx-web"]
