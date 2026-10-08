use dbx_core::database_capabilities::{is_metadata_connection_scoped, is_single_connection_pool, skips_tcp_probe};
use dbx_core::models::connection::DatabaseType;
#[test]
fn mysql_manifest_matches_native_pool_behavior() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../assets/database-drivers.manifest.json")).unwrap();
    let drivers = manifest["drivers"].as_array().unwrap();
    assert_eq!(drivers.len(), 1);
    assert_eq!(drivers[0]["dbType"], "mysql");
    assert_eq!(drivers[0]["runtimeMode"], "native");
    assert!(!is_single_connection_pool(&DatabaseType::Mysql));
    assert!(is_metadata_connection_scoped(&DatabaseType::Mysql));
    assert!(!skips_tcp_probe(&DatabaseType::Mysql));
}
#[test]
fn database_type_rejects_retired_engines() {
    assert_eq!(serde_json::from_str::<DatabaseType>("\"mysql\"").unwrap(), DatabaseType::Mysql);
    for engine in ["postgres", "sqlite", "redis", "mongo", "oracle", "sqlserver", "tidb", "mariadb", "plugin"] {
        assert!(serde_json::from_value::<DatabaseType>(serde_json::json!(engine)).is_err(), "accepted {engine}");
    }
}
