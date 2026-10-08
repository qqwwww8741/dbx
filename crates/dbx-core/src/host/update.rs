pub use dbx_platform::proxy::system_proxy_url;
pub use dbx_platform::version::{is_newer_version, normalize_version, parse_version};
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub portable_mode: bool,
    pub manual_update_only: bool,
    pub release_name: String,
    pub release_url: String,
    pub release_notes: String,
}
pub fn manual_update_info(version: &str) -> UpdateInfo {
    UpdateInfo {
        current_version: version.into(),
        latest_version: version.into(),
        update_available: false,
        portable_mode: false,
        manual_update_only: true,
        release_name: "DBX MySQL".into(),
        release_url: "https://github.com/qqwwww8741/dbx/releases".into(),
        release_notes: String::new(),
    }
}
