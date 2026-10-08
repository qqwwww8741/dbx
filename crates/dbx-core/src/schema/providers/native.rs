use crate::connection::{MysqlMode, PoolKind};
use crate::db;
use crate::models::connection::{ConnectionConfig, DatabaseType};

pub(in crate::schema) async fn list_databases(
    pool: &PoolKind,
    config: Option<&ConnectionConfig>,
) -> Result<Vec<db::DatabaseInfo>, String> {
    match pool {
        PoolKind::Mysql(p, _) => db::mysql::list_databases(p).await,

        _ => Ok(vec![]),
    }
}

pub(in crate::schema) async fn list_schemas(pool: &PoolKind) -> Result<Vec<String>, String> {
    match pool {
        _ => Ok(vec![]),
    }
}

pub(in crate::schema) async fn list_tables(
    pool: &PoolKind,
    config: Option<&ConnectionConfig>,
    database: &str,
    schema: &str,
) -> Result<Vec<db::TableInfo>, String> {
    match pool {
        PoolKind::Mysql(p, _) => {
            let db = if schema.is_empty() { database } else { schema };
            db::mysql::list_tables(p, db).await
        }

        _ => Ok(vec![]),
    }
}

pub(in crate::schema) async fn list_objects(
    pool: &PoolKind,
    config: Option<&ConnectionConfig>,
    database: &str,
    schema: &str,
) -> Result<Option<Vec<db::ObjectInfo>>, String> {
    match pool {
        PoolKind::Mysql(p, _) => {
            db::mysql::list_objects(p, database, None, None, None).await.map(|result| Some(result.objects))
        }

        _ => Ok(None),
    }
}

pub(in crate::schema) async fn list_completion_objects(
    pool: &PoolKind,
    config: Option<&ConnectionConfig>,
    database: &str,
    schema: &str,
) -> Result<Option<Vec<db::ObjectInfo>>, String> {
    match pool {
        PoolKind::Mysql(p, mode) if true => db::mysql::list_completion_objects(p, database).await.map(Some),

        _ => Ok(None),
    }
}

pub(in crate::schema) async fn get_columns(
    pool: &PoolKind,
    config: Option<&ConnectionConfig>,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::ColumnInfo>, String> {
    match pool {
        PoolKind::Mysql(p, _) => db::mysql::get_columns(p, database, table).await,

        _ => Ok(vec![]),
    }
}

pub(in crate::schema) async fn list_indexes(
    pool: &PoolKind,
    config: Option<&ConnectionConfig>,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::IndexInfo>, String> {
    match pool {
        PoolKind::Mysql(p, _) => db::mysql::list_indexes(p, schema, table).await,

        _ => Ok(vec![]),
    }
}

pub(in crate::schema) async fn list_foreign_keys(
    pool: &PoolKind,
    schema: &str,
    table: &str,
) -> Result<Vec<db::ForeignKeyInfo>, String> {
    match pool {
        PoolKind::Mysql(p, _) => db::mysql::list_foreign_keys(p, schema, table).await,

        _ => Ok(vec![]),
    }
}

pub(in crate::schema) async fn list_triggers(
    pool: &PoolKind,
    schema: &str,
    table: &str,
) -> Result<Vec<db::TriggerInfo>, String> {
    match pool {
        PoolKind::Mysql(p, _) => db::mysql::list_triggers(p, schema, table).await,

        _ => Ok(vec![]),
    }
}

pub(in crate::schema) async fn table_ddl(
    pool: &PoolKind,
    config: Option<&ConnectionConfig>,
    schema: &str,
    table: &str,
) -> Result<String, String> {
    match pool {
        PoolKind::Mysql(p, _) => super::super::mysql_ddl(p, schema, table).await,

        _ => Err("DDL not supported for this database type".to_string()),
    }
}

pub(in crate::schema) async fn object_source(
    pool: &PoolKind,
    database: &str,
    schema: &str,
    name: &str,
    object_type: &db::ObjectSourceKind,
) -> Result<Option<String>, String> {
    match pool {
        PoolKind::Mysql(pool, _) => super::super::mysql_object_source(
            pool,
            super::super::mysql_table_metadata_catalog(database, schema),
            name,
            object_type,
        )
        .await
        .map(Some),

        _ => Ok(None),
    }
}

fn collection_names_to_tables(names: Vec<String>, table_type: &str) -> Vec<db::TableInfo> {
    names
        .into_iter()
        .map(|name| db::TableInfo {
            name,
            table_type: table_type.to_string(),
            valid: None,
            comment: None,
            parent_schema: None,
            parent_name: None,
        })
        .collect()
}

fn is_cloudberry_config(config: &ConnectionConfig) -> bool {
    matches!(config.driver_profile.as_deref(), Some("cloudberry"))
}

fn mysql_show_metadata_database_for_config<'a>(config: Option<&ConnectionConfig>, database: &'a str) -> &'a str {
    {
        database
    }
}

fn filter_mysql_system_databases_for_config(
    databases: Vec<db::DatabaseInfo>,
    config: Option<&ConnectionConfig>,
) -> Vec<db::DatabaseInfo> {
    {
        return databases;
    }
}

fn is_mysql_system_database(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "information_schema" | "mysql" | "performance_schema" | "sys")
}

fn is_questdb_config(config: &ConnectionConfig) -> bool {
    false
}
