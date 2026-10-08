pub use dbx_core::update::UpdateInfo;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
#[derive(Default)]
pub struct PendingUpdateState;
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateDownloadSource {
    Official,
    Cnb,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadedUpdate {
    pub cache_id: String,
    pub version: String,
    pub portable_mode: bool,
    pub release_url: String,
    pub release_notes: String,
    pub downloaded_at: u64,
}
const MANUAL_UPDATE: &str = "Download MySQL edition releases from https://github.com/qqwwww8741/dbx/releases";
#[tauri::command]
pub async fn check_for_updates(
    locale: Option<String>,
    source: Option<dbx_core::DownloadSource>,
) -> Result<UpdateInfo, String> {
    let mut info = dbx_core::update::manual_update_info(env!("CARGO_PKG_VERSION"));
    info.portable_mode = crate::data_dir::is_portable_mode();
    Ok(info)
}
#[tauri::command]
pub async fn fetch_changelog(lang: Option<String>) -> Result<dbx_core::changelog::ChangelogData, String> {
    dbx_core::changelog::fetch_changelog(&lang.unwrap_or_else(|| "en".into())).await
}
#[tauri::command]
pub async fn get_system_proxy_url() -> Option<String> {
    tauri::async_runtime::spawn_blocking(dbx_core::update::system_proxy_url).await.ok().flatten()
}
#[tauri::command]
pub fn cancel_update_download(_state: tauri::State<'_, PendingUpdateState>) {}
#[tauri::command]
pub fn get_downloaded_update(
    _app: AppHandle,
    _state: tauri::State<'_, PendingUpdateState>,
) -> Result<Option<DownloadedUpdate>, String> {
    Ok(None)
}
#[tauri::command]
pub fn discard_downloaded_update(
    _app: AppHandle,
    _state: tauri::State<'_, PendingUpdateState>,
    cache_id: String,
) -> Result<(), String> {
    Ok(())
}
#[tauri::command]
pub async fn download_update(
    _app: AppHandle,
    _state: tauri::State<'_, PendingUpdateState>,
    source: UpdateDownloadSource,
    latest_version: String,
    attempt_id: String,
    release_notes: Option<String>,
) -> Result<DownloadedUpdate, String> {
    Err(MANUAL_UPDATE.into())
}
#[tauri::command]
pub fn install_downloaded_update(
    _app: AppHandle,
    _state: tauri::State<'_, PendingUpdateState>,
    cache_id: String,
    expected_version: String,
) -> Result<(), String> {
    Err(MANUAL_UPDATE.into())
}
