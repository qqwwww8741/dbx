use std::sync::Arc;

use axum::{extract::Query, extract::State, Json};
use dbx_core::{changelog, update};

use crate::error::AppError;
use crate::state::WebState;

pub async fn get_version(State(state): State<Arc<WebState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "demoMode": state.demo_mode }))
}

#[derive(serde::Deserialize)]
pub struct UpdateCheckParams {
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub source: Option<dbx_core::DownloadSource>,
}

pub async fn check_for_updates(Query(params): Query<UpdateCheckParams>) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(
        serde_json::to_value(update::manual_update_info(env!("CARGO_PKG_VERSION")))
            .map_err(|e| AppError::from(e.to_string()))?,
    ))
}

#[derive(serde::Deserialize)]
pub struct ChangelogParams {
    #[serde(default)]
    pub lang: Option<String>,
}

pub async fn fetch_changelog(
    Query(params): Query<ChangelogParams>,
) -> Result<Json<changelog::ChangelogData>, AppError> {
    let lang = params.lang.unwrap_or_else(|| "en".to_string());
    let data = changelog::fetch_changelog(&lang).await.map_err(AppError::from)?;
    Ok(Json(data))
}
