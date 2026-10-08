use crate::{database_manifest, models::connection::DatabaseType};
pub fn is_single_connection_pool(db_type: &DatabaseType) -> bool {
    database_manifest::entry(db_type).is_some_and(|entry| entry.single_connection_pool)
}
pub fn is_metadata_connection_scoped(db_type: &DatabaseType) -> bool {
    database_manifest::entry(db_type).is_some_and(|entry| entry.metadata_connection_scoped)
}
pub fn skips_tcp_probe(db_type: &DatabaseType) -> bool {
    database_manifest::entry(db_type).is_some_and(|entry| entry.skip_tcp_probe)
}
pub fn is_local_file_db_type(db_type: &DatabaseType) -> bool {
    database_manifest::entry(db_type).is_some_and(|entry| entry.local_file)
}
