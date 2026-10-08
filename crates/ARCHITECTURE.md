# MySQL edition workspace

- `dbx-types`: shared API types and MySQL connection configuration.
- `dbx-driver-support`, `dbx-driver-mysql`, `dbx-drivers`: native MySQL connectivity, execution, SSH, and shared errors.
- `dbx-sql-core`, `dbx-sql-dialect`, `dbx-sql-schema`, `dbx-sql-data`, `dbx-sql`: MySQL parsing, schema operations, data transfer, and SQL generation.
- `dbx-core`: application state, internal SQLite storage, query execution, import/export, backup scheduling, AI tools, and application plugins.
- `dbx-web`, `src-tauri`: HTTP and desktop adapters over the shared core.
- `dbx-cli`, `dbx-mcp`: command line and MCP clients of the shared core.
- `dbx-platform`, `dbx-formats`, `dbx-plugin-runtime`, `dbx-ai-provider`, `dbx-tauri-schema`: platform support and shared app infrastructure.

Only the native MySQL driver is exposed as a database connection. Bundled SQLite and optional SQLCipher features are for application storage.

Connection descriptors live under `plugins/connection-types`; `scripts/sync-connection-types.mjs` checks the generated frontend types against them. See [CONTRIBUTING.md](../CONTRIBUTING.md) for validation commands.
