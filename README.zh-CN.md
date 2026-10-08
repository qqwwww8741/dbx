# DBX MySQL 专用版

基于 [t8y2/dbx](https://github.com/t8y2/dbx) v0.6.36 的 MySQL 专用分支。

仅支持 MySQL 原生驱动。保留桌面端、Web、Docker、CLI、AI、MCP，以及 SQL 编辑、表结构浏览和编辑、数据编辑、导入导出、备份、SSH／代理／HTTP 隧道。

其他数据库、中间件的驱动、连接选项、专用页面、运行时安装器和部署模板已移除。SQLite 仅用于程序自身的本地配置与历史记录存储，不作为可连接数据库提供。

## 开发与构建

需要 Node.js 22.13+、pnpm 10.27.0、稳定版 Rust，以及对应系统的 Tauri 构建依赖。

```sh
pnpm install --frozen-lockfile
pnpm dev:tauri
```

Web 开发分别启动后端和前端：

```sh
cargo run -p dbx-web
pnpm dev:web
```

检查与构建：

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

访问 http://localhost:4224。对外提供服务前请修改 Compose 中的 `DBX_PASSWORD`。配置与备份通过命名卷保存。

[MySQL 连接说明](docs/mysql.md)。本分支不使用原版 DBX 的自动更新源。

## 许可证

Apache-2.0。保留原项目及贡献者的版权声明，详见 [LICENSE](LICENSE)。

## 原生 MCP

```sh
cargo build --release -p dbx-mcp --features dbx-core/sqlite-sqlcipher,os-keyring
```

将 `target/release` 加入 PATH，然后在设置中刷新 MCP 状态。可用 `dbx-mcp --mysql-version` 确认运行的是本分支的 MySQL 版本。
