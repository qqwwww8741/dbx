//! Read-only tests against explicitly configured disposable fixtures.
//! Doris is related-engine evidence, not an exact SelectDB 3.0.11 reproduction.
use dbx_drivers::db::mysql;
use dbx_drivers::types::{CompletionAssistantObjectKind, CompletionAssistantRequest};
use serde_json::json;
use std::time::Duration;

fn request(database: &str, mask: &str, kinds: &[&str], limit: usize) -> CompletionAssistantRequest {
    serde_json::from_value(json!({
        "connection_id": "live-routine-completion", "database": database, "schema": database,
        "object_kinds": kinds, "mask": mask, "max_results": limit, "match_mode": "prefix"
    }))
    .unwrap()
}

#[tokio::test]
#[ignore = "requires DBX_LIVE_MYSQL_ROUTINES_URL and DATABASE with dbx_probe_fn(INT) RETURNS INT fixture"]
async fn live_mysql_routine_completion_keeps_primary_base_type() {
    let url = std::env::var("DBX_LIVE_MYSQL_ROUTINES_URL").expect("DBX_LIVE_MYSQL_ROUTINES_URL");
    let database = std::env::var("DBX_LIVE_MYSQL_ROUTINES_DATABASE").expect("DBX_LIVE_MYSQL_ROUTINES_DATABASE");
    let pool = mysql::connect(&url, Duration::from_secs(15)).await.unwrap();
    let query = request(&database, "dbx_probe_", &["function"], 200);
    let response = mysql::completion_assistant_search(&pool, &query).await.unwrap();
    assert!(!response.fallback_used);
    let function = response.candidates.iter().find(|candidate| candidate.name == "dbx_probe_fn").unwrap();
    assert_eq!(function.data_type.as_deref(), Some("int"));
    assert_eq!(function.schema.as_deref(), Some(database.as_str()));
    let mixed =
        mysql::completion_assistant_search(&pool, &request(&database, "%", &["schema", "table", "function"], 1000))
            .await
            .unwrap();
    assert!(!mixed.fallback_used);
    assert!(mixed.candidates.iter().any(|candidate| candidate.name == database));
    assert!(mixed.candidates.iter().any(|candidate| candidate.name == "dbx_probe_fn"));
    pool.disconnect().await.unwrap();
}
