# Contributing to DBX for MySQL

This fork supports MySQL through the native Rust driver. Keep connection descriptors, generated types, frontend behavior, and Rust handlers aligned when changing connection features.

See [README.md](README.md) for development and build commands. Open changes against [this fork](https://github.com/qqwwww8741/dbx).

Before submitting a change, run:

```sh
pnpm check:connection-types
pnpm typecheck
pnpm build
pnpm test:db-env
cargo check --workspace --all-targets --no-default-features --features dbx-core/sqlite-bundled
cargo test -p dbx-driver-mysql -p dbx-sql-core -p dbx-sql-data -p dbx-sql-dialect -p dbx-sql-schema -p dbx-types --lib
```

Live integration tests under `crates/dbx-core/tests/live_mysql_*.rs` require a disposable MySQL server; each test lists its environment variables. They create and remove temporary databases. SQLite is used internally for application settings and history.

Keep Apache-2.0 notices and the original contributors' copyright notices.
