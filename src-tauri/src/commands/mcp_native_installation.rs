use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
pub(super) struct NativeInstallation {
    pub binary: PathBuf,
    pub version: Option<String>,
}
fn home_directory() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}
fn standalone_bin_directory(home: &Path) -> PathBuf {
    home.join(".dbx").join("bin")
}
pub(super) fn resolve() -> Option<NativeInstallation> {
    let mut dirs =
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).into_iter().collect::<Vec<_>>();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    if let Some(home) = home_directory() {
        dirs.push(standalone_bin_directory(&home));
    }
    dirs.into_iter().find_map(|dir| {
        let binary = dir.join(if cfg!(windows) { "dbx-mcp.exe" } else { "dbx-mcp" });
        if !binary.is_absolute() || !binary.is_file() {
            return None;
        }
        let version = binary_version(&binary)?;
        Some(NativeInstallation { binary, version: Some(version) })
    })
}
fn parse_version(value: &str) -> Option<String> {
    let value = value.trim().strip_prefix("dbx-mcp-mysql ")?;
    semver::Version::parse(value).ok().map(|_| value.to_string())
}
fn binary_version(binary: &Path) -> Option<String> {
    let mut child = dbx_core::process::new_std_command(binary)
        .arg("--mysql-version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let output = child.wait_with_output().ok()?;
                return parse_version(std::str::from_utf8(&output.stdout).ok()?);
            }
            Ok(Some(_)) => return None,
            Ok(None) if started.elapsed() < Duration::from_secs(2) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

pub(super) fn install_command() -> &'static str {
    "cargo build --release -p dbx-mcp --features dbx-core/sqlite-sqlcipher,os-keyring"
}
impl NativeInstallation {
    pub fn uninstall_command(&self) -> String {
        format!("Remove the MySQL MCP binary at {}", self.binary.display())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_upstream_version() {
        assert_eq!(parse_version("dbx-mcp 0.6.36"), None);
        assert_eq!(parse_version("dbx-mcp-mysql 0.6.36"), Some("0.6.36".into()));
    }
}
