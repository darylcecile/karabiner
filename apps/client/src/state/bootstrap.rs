//! Managed local service: when the default service isn't answering, use (or fetch) the
//! service binary that matches this client build and start it in the background.
//!
//! - Only for the default local service. `KARABINER_SERVICE_URL` means an external service
//!   the user runs; it is never replaced or started here.
//! - Builds name their service with `KARABINER_SERVICE_ASSET_URL` at compile time (a release
//!   asset for the same version, plus `<url>.sha256`). Dev builds without it keep the manual
//!   "start it yourself" message.
//! - The binary is cached under `~/Library/Application Support/Karabiner/service/<key>/`,
//!   checked against its SHA-256 before every launch, written atomically and executable by
//!   the user only. A failed download leaves any existing copy untouched.
//! - The started service keeps running after the window closes (it owns runs and the DB);
//!   nothing is installed as a login item or daemon.

use std::{
    fs,
    io::Write as _,
    os::unix::{fs::PermissionsExt as _, process::CommandExt as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use sha2::{Digest as _, Sha256};

use crate::api::ApiClient;

const BINARY: &str = "karabiner-service";
const CHECKSUM: &str = "karabiner-service.sha256";
/// Generous bound for the compiled Bun service (tens of MB).
const MAX_BINARY: u64 = 256 * 1024 * 1024;
const MAX_CHECKSUM: u64 = 4096;

/// The release asset to manage, or `None` when the service is external or no asset is
/// configured (dev builds). `KARABINER_SERVICE_ASSET_URL` at run time overrides the build's
/// (used to test setup against a local copy). Only HTTPS, or HTTP on this machine.
pub fn asset_url() -> Option<String> {
    if std::env::var_os("KARABINER_SERVICE_URL").is_some() {
        return None;
    }
    let url = std::env::var("KARABINER_SERVICE_ASSET_URL").ok()
        .or_else(|| option_env!("KARABINER_SERVICE_ASSET_URL").map(str::to_string))
        .filter(|url| !url.is_empty())?;
    let allowed = url.starts_with("https://") || url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:");
    allowed.then_some(url)
}

/// The managed service's port: `KARABINER_PORT` (the service's own setting) or the default.
/// The client talks to `127.0.0.1:<port>` and starts the service there.
pub fn managed_port() -> u16 {
    std::env::var("KARABINER_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or_else(default_port)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// `~/Library/Application Support/Karabiner`, or `KARABINER_SUPPORT_DIR` (keeps test runs'
/// cache and logs apart from the user's).
fn support_root() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("KARABINER_SUPPORT_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").ok_or("HOME isn't set, so the service can't be stored.")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Karabiner"))
}

/// One cache folder per asset URL (it names the release), so each version keeps its own.
fn cache_dir(root: &Path, url: &str) -> PathBuf {
    root.join("service").join(&sha256(url.as_bytes())[..16])
}

/// The cached binary if it's present and still matches its recorded checksum.
pub fn cached(url: &str) -> Option<PathBuf> {
    cached_in(&support_root().ok()?, url)
}

fn cached_in(root: &Path, url: &str) -> Option<PathBuf> {
    let dir = cache_dir(root, url);
    let expected = fs::read_to_string(dir.join(CHECKSUM)).ok()?;
    let binary = dir.join(BINARY);
    (sha256(&fs::read(&binary).ok()?) == expected.trim()).then_some(binary)
}

/// First hex token of a `shasum -a 256` line.
fn parse_checksum(text: &[u8]) -> Result<String, String> {
    let token = std::str::from_utf8(text).ok().and_then(|text| text.split_whitespace().next()).unwrap_or_default();
    let valid = token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit());
    if valid { Ok(token.to_ascii_lowercase()) } else { Err("The service checksum file isn't valid.".into()) }
}

/// Writes `bytes` to `path` via a temporary file and rename, with the given mode.
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let temporary = path.with_extension("partial");
    let write = || -> std::io::Result<()> {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
        fs::rename(&temporary, path)
    };
    write().map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("Couldn't save the service: {error}")
    })
}

/// Downloads the binary and its expected checksum (verify and save with [`install`]).
pub async fn fetch(api: &ApiClient, url: &str) -> Result<(Vec<u8>, String), String> {
    let failed = |error: crate::api::ApiError| format!("Couldn't download the Karabiner service ({error}). Check your connection, then retry.");
    let expected = parse_checksum(&api.download(&format!("{url}.sha256"), MAX_CHECKSUM).await.map_err(failed)?)?;
    let bytes = api.download(url, MAX_BINARY).await.map_err(failed)?;
    Ok((bytes, expected))
}

/// Verifies a download and saves it to the cache; nothing is written if it doesn't match.
pub fn install(url: &str, bytes: &[u8], expected: &str) -> Result<PathBuf, String> {
    install_in(&support_root()?, url, bytes, expected)
}

fn install_in(root: &Path, url: &str, bytes: &[u8], expected: &str) -> Result<PathBuf, String> {
    if sha256(bytes) != expected {
        return Err("The downloaded service didn't match its checksum, so it wasn't run. Retry to download it again.".into());
    }
    let dir = cache_dir(root, url);
    fs::create_dir_all(&dir).map_err(|error| format!("Couldn't create {}: {error}", dir.display()))?;
    let binary = dir.join(BINARY);
    write_atomic(&binary, bytes, 0o700)?;
    write_atomic(&dir.join(CHECKSUM), expected.as_bytes(), 0o600)?;
    Ok(binary)
}

/// Starts the service on the default port in its own process group, logging to
/// `logs/service.log`. Returns the log path for error messages.
pub fn launch(binary: &Path, port: u16) -> Result<(std::process::Child, PathBuf), String> {
    let logs = support_root()?.join("logs");
    fs::create_dir_all(&logs).map_err(|error| format!("Couldn't create {}: {error}", logs.display()))?;
    let log_path = logs.join("service.log");
    let log = fs::OpenOptions::new().create(true).append(true).open(&log_path)
        .map_err(|error| format!("Couldn't open {}: {error}", log_path.display()))?;
    let error_log = log.try_clone().map_err(|error| error.to_string())?;
    let home = std::env::var("HOME").unwrap_or_default();
    let path = service_path(&std::env::var("PATH").unwrap_or_default(), &home);
    let child = Command::new(binary)
        .args(["--port", &port.to_string()])
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(error_log)
        // Its own group: quitting the app (or a terminal signal) doesn't stop running work.
        .process_group(0)
        .spawn()
        .map_err(|error| format!("Couldn't start the Karabiner service: {error}"))?;
    Ok((child, log_path))
}

/// Where the usual installers put CLIs (Claude/Copilot/Codex installers, Bun, Homebrew).
/// A Finder-launched app gets a minimal PATH, so the service couldn't find harnesses or `gh`.
const INSTALL_DIRS: [&str; 5] = ["~/.local/bin", "~/.bun/bin", "/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"];

/// The inherited PATH, in its order, plus any standard install folder it lacks. No shell
/// profile is read.
fn service_path(inherited: &str, home: &str) -> String {
    let mut entries: Vec<String> = inherited.split(':').filter(|entry| !entry.is_empty()).map(str::to_string).collect();
    for dir in INSTALL_DIRS {
        let dir = dir.replacen('~', home, 1);
        if !entries.contains(&dir) { entries.push(dir) }
    }
    entries.join(":")
}

/// The port of the default service URL (the managed service only runs there).
pub fn default_port() -> u16 {
    protocol::DEFAULT_SERVICE_URL.rsplit(':').next().and_then(|port| port.parse().ok()).unwrap_or(43173)
}

#[cfg(test)]
mod tests {
    use std::fs;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("karabiner-bootstrap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    /// A mismatched download is never saved and doesn't disturb a good cached copy; a
    /// tampered cached copy isn't used.
    #[test]
    fn only_verified_binaries_are_saved_and_run() {
        let (root, url) = (temp_root("verify"), "https://example.test/v1/karabiner-service-macos-arm64");
        let good = b"service v1";
        let installed = super::install_in(&root, url, good, &super::sha256(good)).unwrap();
        assert_eq!(super::cached_in(&root, url), Some(installed.clone()));
        assert!(super::install_in(&root, url, b"tampered", &super::sha256(good)).is_err());
        assert_eq!(fs::read(&installed).unwrap(), good);
        fs::write(&installed, b"tampered").unwrap();
        assert_eq!(super::cached_in(&root, url), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn service_path_keeps_order_and_adds_missing_install_folders() {
        let path = super::service_path("/usr/bin:/bin:/opt/homebrew/bin", "/Users/me");
        assert_eq!(path, "/usr/bin:/bin:/opt/homebrew/bin:/Users/me/.local/bin:/Users/me/.bun/bin:/opt/homebrew/sbin:/usr/local/bin");
    }

    #[test]
    fn checksum_lines_are_validated() {
        let line = format!("{}  karabiner-service-macos-arm64\n", "A".repeat(64));
        assert_eq!(super::parse_checksum(line.as_bytes()).unwrap(), "a".repeat(64));
        assert!(super::parse_checksum(b"not-a-hash  file").is_err());
    }
}
