use crate::sql_dialect::descriptor::{DialectCapabilityDescriptor, DialectInfo, DialectKind, TypeMappingMatrix};
use insta::assert_json_snapshot;

#[test]
fn snapshot_mysql_descriptor() {
    let desc = DialectCapabilityDescriptor::for_dialect(DialectKind::Mysql);
    assert_json_snapshot!("mysql_descriptor", desc);
}

#[test]
fn snapshot_mysql_info() {
    let info = DialectInfo::for_kind(DialectKind::Mysql);
    assert_json_snapshot!("mysql_info", info);
}

#[test]
fn snapshot_all_dialects_info() {
    let all = DialectInfo::all();
    assert_json_snapshot!("all_dialects_info", all);
}
