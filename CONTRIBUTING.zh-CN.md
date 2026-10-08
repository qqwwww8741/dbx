# 参与 DBX MySQL 专用版开发

本分支通过原生 Rust 驱动支持 MySQL。修改连接功能时，请同步维护连接描述、生成类型、前端行为和 Rust 接口。

开发和构建命令见 [README.zh-CN.md](README.zh-CN.md)。请向[当前仓库](https://github.com/qqwwww8741/dbx)提交修改。

提交前运行：

```sh
pnpm check:connection-types
pnpm typecheck
pnpm build
pnpm test:db-env
cargo check --workspace --all-targets --no-default-features --features dbx-core/sqlite-bundled
cargo test -p dbx-driver-mysql -p dbx-sql-core -p dbx-sql-data -p dbx-sql-dialect -p dbx-sql-schema -p dbx-types --lib
```

`crates/dbx-core/tests/live_mysql_*.rs` 中的实库测试需要可随时清空的 MySQL 测试实例，各测试注明所需环境变量，并创建、删除临时数据库。SQLite 仅用于程序内部配置和历史记录存储。

请保留 Apache-2.0 许可及原贡献者的版权声明。
