#[path = "mcp_native_installation.rs"]
mod native_installation;
use serde::Serialize;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
const SHELL_COMMAND_MARKER: &str = "__DBX_MCP_COMMAND_OUTPUT_START__";
#[derive(Debug, Serialize)]
pub struct McpServerStatus {
    pub installed: bool,
    pub installation_source: Option<String>,
    pub npm_available: bool,
    pub npm_installed: bool,
    pub node_path: Option<String>,
    pub node_version: Option<String>,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub bin_path: Option<String>,
    pub native_bin_path: Option<String>,
    pub script_path: Option<String>,
    pub data_dir: Option<String>,
    pub install_command: String,
    pub update_command: String,
    pub uninstall_command: String,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn check_mcp_server_status(app: AppHandle) -> Result<McpServerStatus, String> {
    let default_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let data_dir = crate::data_dir::resolve_data_dir_with_mode(default_data_dir).custom_data_dir().map(path_string);
    let native = tauri::async_runtime::spawn_blocking(native_installation::resolve).await.map_err(|e| e.to_string())?;
    let bin_path = native.as_ref().map(|n| path_string(&n.binary));
    Ok(McpServerStatus {
        installed: native.is_some(),
        installation_source: native.as_ref().map(|_| "native".into()),
        npm_available: false,
        npm_installed: false,
        node_path: None,
        node_version: None,
        current_version: native.as_ref().and_then(|n| n.version.clone()),
        latest_version: None,
        update_available: false,
        native_bin_path: bin_path.clone(),
        bin_path,
        script_path: None,
        data_dir,
        install_command: native_installation::install_command().into(),
        update_command: native_installation::install_command().into(),
        uninstall_command: native.as_ref().map(|n| n.uninstall_command()).unwrap_or_default(),
        error: None,
    })
}

pub(crate) async fn resolve_mcp_server_command() -> Result<(String, Vec<String>), String> {
    tauri::async_runtime::spawn_blocking(||native_installation::resolve().map(|n|(path_string(&n.binary),Vec::new())).ok_or_else(||"[dbxMcpMissing] Build the MySQL MCP server with cargo build --release -p dbx-mcp --features dbx-core/sqlite-sqlcipher,os-keyring and add its directory to PATH.".to_string())).await.map_err(|e|e.to_string())?
}
fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[allow(clippy::needless_return)]
pub(crate) fn locate_command(command: &str) -> Option<String> {
    #[cfg(windows)]
    {
        locate_windows_command(command)
    }
    #[cfg(not(windows))]
    {
        command_stdout("which", &[command]).ok().and_then(first_non_empty_line)
    }
}

#[cfg(windows)]
fn locate_windows_command(command: &str) -> Option<String> {
    command_stdout("where", &[command])
        .ok()
        .and_then(first_windows_command_path)
        .or_else(|| {
            let script =
                format!("(Get-Command -All {} -ErrorAction SilentlyContinue).Source", windows_shell_quote(command));
            command_stdout("powershell.exe", &["-NoProfile", "-Command", &script])
                .ok()
                .and_then(first_windows_command_path)
        })
        .or_else(|| {
            windows_command_candidates(command)
                .into_iter()
                .find(|candidate| is_windows_launchable_command(candidate) && Path::new(candidate).is_file())
        })
}

#[cfg(windows)]
fn first_windows_command_path(value: String) -> Option<String> {
    let paths = value.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>();
    paths
        .into_iter()
        .find(|path| is_windows_launchable_command(path) && Path::new(path).is_file())
        .map(ToOwned::to_owned)
}

#[cfg(windows)]
fn is_windows_launchable_command(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("exe" | "cmd" | "bat" | "com")
    )
}

fn command_stdout(command: &str, args: &[&str]) -> Result<String, String> {
    let output = command_output(command, args)?;
    if !output.success {
        return Err(output.stderr.trim().to_string());
    }
    Ok(output.stdout.trim().to_string())
}

fn first_non_empty_line(value: String) -> Option<String> {
    value.lines().map(str::trim).find(|line| !line.is_empty()).map(ToOwned::to_owned)
}

#[derive(Debug)]
struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

#[allow(clippy::needless_return)]
fn command_output(command: &str, args: &[&str]) -> Result<CommandOutput, String> {
    let direct = run_command(command, args);
    if direct.as_ref().is_ok_and(|output| output.success) {
        return direct;
    }

    #[cfg(windows)]
    {
        run_windows_command_candidates(command, args).or(direct)
    }

    #[cfg(not(windows))]
    {
        run_command_through_user_shell(command, args).or(direct)
    }
}

fn run_command<I, S>(command: impl AsRef<OsStr>, args: I) -> Result<CommandOutput, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = dbx_core::process::new_std_command(command);
    cmd.args(args);
    command_output_from_process(cmd)
}

fn command_output_from_process(mut command: std::process::Command) -> Result<CommandOutput, String> {
    let output = command.output().map_err(|e| e.to_string())?;
    Ok(CommandOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

#[cfg(windows)]
fn run_windows_command_candidates(command: &str, args: &[&str]) -> Result<CommandOutput, String> {
    for candidate in windows_command_candidates(command) {
        let output = run_command(&candidate, args);
        if output.as_ref().is_ok_and(|output| output.success) {
            return output;
        }
    }
    run_command_through_user_shell(command, args)
}

#[cfg(windows)]
fn windows_command_candidates(command: &str) -> Vec<String> {
    if Path::new(command).extension().is_some() {
        return Vec::new();
    }
    let names = ["cmd", "exe", "bat", "com", "ps1"].iter().map(|extension| format!("{command}.{extension}"));
    names
        .clone()
        .chain(
            windows_common_command_dirs()
                .into_iter()
                .flat_map(|dir| names.clone().map(move |name| dir.join(name).to_string_lossy().to_string())),
        )
        .collect()
}

#[cfg(windows)]
fn windows_common_command_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(nvm_symlink) = std::env::var("NVM_SYMLINK") {
        dirs.push(nvm_symlink.into());
    }
    if let Ok(app_data) = std::env::var("APPDATA") {
        dirs.push(std::path::PathBuf::from(app_data).join("npm"));
    }
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        dirs.push(std::path::PathBuf::from(program_files).join("nodejs"));
    }
    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        dirs.push(std::path::PathBuf::from(program_files_x86).join("nodejs"));
    }
    dirs.push(std::path::PathBuf::from(r"C:\nvm4w\nodejs"));
    dirs
}

#[cfg(windows)]
fn run_command_through_user_shell(command: &str, args: &[&str]) -> Result<CommandOutput, String> {
    let script = windows_command_script(command, args);
    let mut output = run_command("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])?;
    output.stdout = stdout_after_shell_marker(&output.stdout);
    Ok(output)
}

#[cfg(windows)]
fn windows_command_script(command: &str, args: &[&str]) -> String {
    let mut words = Vec::with_capacity(args.len() + 1);
    words.push(windows_shell_quote(command));
    words.extend(args.iter().map(|arg| windows_shell_quote(arg)));
    format!("Write-Output {}; & {}", windows_shell_quote(SHELL_COMMAND_MARKER), words.join(" "))
}

#[cfg(windows)]
fn windows_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(not(windows))]
fn run_command_through_user_shell(command: &str, args: &[&str]) -> Result<CommandOutput, String> {
    let script = shell_command_script(command, args);
    let (shell, shell_args) = user_shell_invocation_args(&script);
    let shell_arg_refs = shell_args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut output = run_command(&shell, &shell_arg_refs)?;
    output.stdout = stdout_after_shell_marker(&output.stdout);
    Ok(output)
}

#[cfg(not(windows))]
fn user_shell_invocation_args(script: &str) -> (String, Vec<String>) {
    let shell = env::var("SHELL").ok().filter(|value| !value.trim().is_empty()).unwrap_or_else(default_user_shell);
    let shell_name = Path::new(&shell).file_name().and_then(|value| value.to_str()).unwrap_or_default();
    let args = match shell_name {
        "fish" => vec!["-l".to_string(), "-i".to_string(), "-c".to_string(), script.to_string()],
        "bash" => vec![
            "--noprofile".to_string(),
            "--norc".to_string(),
            "-i".to_string(),
            "-c".to_string(),
            bash_login_script(script),
        ],
        "sh" | "dash" => vec!["-ic".to_string(), script.to_string()],
        "zsh" => vec!["-ilc".to_string(), script.to_string()],
        _ => vec!["-lc".to_string(), script.to_string()],
    };
    (shell, args)
}

#[cfg(not(windows))]
fn bash_login_script(script: &str) -> String {
    format!(
        "for dbx_profile in ~/.bash_profile ~/.bash_login ~/.profile ~/.bashrc; do \
         [ -r \"$dbx_profile\" ] && . \"$dbx_profile\"; \
         done; unset dbx_profile; {script}"
    )
}

#[cfg(not(windows))]
fn default_user_shell() -> String {
    if Path::new("/bin/zsh").exists() {
        "/bin/zsh".to_string()
    } else {
        "/bin/sh".to_string()
    }
}

#[cfg(not(windows))]
fn shell_command_script(command: &str, args: &[&str]) -> String {
    let mut words = Vec::with_capacity(args.len() + 1);
    words.push(shell_quote(command));
    words.extend(args.iter().map(|arg| shell_quote(arg)));
    format!("printf '%s\\n' {}; {}", shell_quote(SHELL_COMMAND_MARKER), words.join(" "))
}

#[cfg(not(windows))]
fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn stdout_after_shell_marker(stdout: &str) -> String {
    stdout
        .find(SHELL_COMMAND_MARKER)
        .map(|index| stdout[index + SHELL_COMMAND_MARKER.len()..].trim_start_matches(['\r', '\n']).to_string())
        .unwrap_or_else(|| stdout.to_string())
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::first_windows_command_path;
    #[cfg(windows)]
    #[test]
    fn windows_command_lookup_prefers_cmd_over_extensionless_shim() {
        let dir = std::env::temp_dir().join(format!("dbx-mcp-command-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let extensionless = dir.join("codex");
        let cmd = dir.join("codex.cmd");
        std::fs::write(&extensionless, "#!/bin/sh\n").unwrap();
        std::fs::write(&cmd, "@echo off\n").unwrap();

        let output = format!("{}\n{}\n", extensionless.display(), cmd.display());
        let resolved = first_windows_command_path(output).unwrap();

        assert_eq!(resolved, cmd.to_string_lossy().as_ref());
        let _ = std::fs::remove_file(extensionless);
        let _ = std::fs::remove_file(cmd);
        let _ = std::fs::remove_dir(dir);
    }
    #[cfg(windows)]
    #[test]
    fn windows_command_lookup_rejects_extensionless_only_shim() {
        let dir = std::env::temp_dir().join(format!("dbx-mcp-command-extensionless-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let extensionless = dir.join("codex");
        std::fs::write(&extensionless, "#!/bin/sh\n").unwrap();

        let resolved = first_windows_command_path(extensionless.display().to_string());

        assert!(resolved.is_none());
        let _ = std::fs::remove_file(extensionless);
        let _ = std::fs::remove_dir(dir);
    }
}
