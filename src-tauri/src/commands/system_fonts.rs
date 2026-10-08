#[tauri::command]
pub async fn list_system_fonts() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(|| dbx_core::host::system_fonts::list_system_fonts())
        .await
        .map_err(|e| e.to_string())
}
