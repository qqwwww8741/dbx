# DBX for MySQL

A MySQL-only fork of [t8y2/dbx](https://github.com/t8y2/dbx), based on v0.6.36.

[简体中文](README.zh-CN.md)

This edition supports MySQL connections through the native Rust driver. It retains the desktop app, Web app, Docker deployment, CLI, AI tools, MCP, SQL editing, schema browsing, data editing, import/export, backups, and SSH/proxy/HTTP tunnels.

Other database and middleware drivers, connection profiles, dedicated interfaces, runtime installers, and deployment recipes have been removed. SQLite remains an internal application storage dependency; it is not a supported connection type.

## Development

Requirements: Node.js 22.13+, pnpm 10.27.0, stable Rust, and the platform build prerequisites for Tauri.

```sh
pnpm install --frozen-lockfile
pnpm dev:tauri
```

For the Web app, run the backend and frontend in separate terminals:

```sh
cargo run -p dbx-web
pnpm dev:web
```

Build and check:

```sh
pnpm check:connection-types
pnpm typecheck
pnpm build
cargo check -p dbx-web -p dbx-cli -p dbx-mcp
```

## Docker

```sh
docker compose -f deploy/docker-compose.yml up --build -d
```

Open http://localhost:4224. Set `DBX_PASSWORD` in the compose environment before exposing the service. Application data and backups are stored in named volumes.

See [the MySQL guide](docs/mysql.md) for connection settings. This fork does not install updates from upstream DBX releases.

## License

Apache-2.0. The original DBX project and its contributors retain their copyrights. See [LICENSE](LICENSE).

## Native MCP

```sh
cargo build --release -p dbx-mcp --features dbx-core/sqlite-sqlcipher,os-keyring
```

Add `target/release` to PATH and refresh MCP status in Settings. The MySQL edition identifies itself with `dbx-mcp --mysql-version`.
