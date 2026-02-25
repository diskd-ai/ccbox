use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const REPO: &str = "diskd-ai/ccbox";
const BIN_NAME: &str = "ccbox";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim().trim_start_matches('v');
        let mut parts = trimmed.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next()?.parse::<u64>().ok()?;
        let patch = parts.next()?.parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateAvailable {
    pub current: Version,
    pub latest: Version,
    pub latest_tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LatestReleaseInfo {
    pub version: Version,
    pub tag: String,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("failed to fetch latest release: {0}")]
    FetchLatest(String),

    #[error("invalid latest release tag: {0}")]
    InvalidLatestTag(String),

    #[error("unsupported platform for self-update: {0}")]
    UnsupportedPlatform(String),

    #[error("failed to download asset: {0}")]
    Download(String),

    #[error("failed to parse sha256 file: {0}")]
    ParseSha(String),

    #[error("sha256 mismatch for {asset}")]
    ShaMismatch { asset: String },

    #[error("failed to extract binary from archive: {0}")]
    Extract(String),

    #[error("failed to locate current executable: {0}")]
    CurrentExe(String),

    #[error("failed to install update to {path}: {source}")]
    Install { path: String, source: io::Error },
}

#[derive(Debug, Deserialize)]
struct GitHubLatestRelease {
    tag_name: String,
}

pub fn check_for_update(current_version: &str) -> Result<Option<UpdateAvailable>, UpdateError> {
    let current = Version::parse(current_version).ok_or_else(|| {
        UpdateError::InvalidLatestTag(format!("current version is invalid: {current_version}"))
    })?;

    let latest = fetch_latest_release_info(Duration::from_secs(4))?;
    if latest.version <= current {
        return Ok(None);
    }

    Ok(Some(UpdateAvailable {
        current,
        latest: latest.version,
        latest_tag: latest.tag,
    }))
}

pub fn fetch_latest_release_info(timeout: Duration) -> Result<LatestReleaseInfo, UpdateError> {
    let agent = make_agent(timeout);

    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut response = agent
        .get(&url)
        .header(
            "User-Agent",
            &format!("{BIN_NAME}/{}", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| UpdateError::FetchLatest(error.to_string()))?;

    let parsed: GitHubLatestRelease = response
        .body_mut()
        .read_json::<GitHubLatestRelease>()
        .map_err(|error| UpdateError::FetchLatest(error.to_string()))?;

    let latest = Version::parse(&parsed.tag_name)
        .ok_or_else(|| UpdateError::InvalidLatestTag(parsed.tag_name.clone()))?;

    Ok(LatestReleaseInfo {
        version: latest,
        tag: parsed.tag_name,
    })
}

pub fn self_update() -> Result<Option<UpdateAvailable>, UpdateError> {
    let Some(update) = check_for_update(env!("CARGO_PKG_VERSION"))? else {
        return Ok(None);
    };

    let archive = download_release_archive(&update.latest_tag, &update.latest)?;
    let extracted = extract_binary_from_tar_gz(&archive).map_err(UpdateError::Extract)?;

    install_binary(&extracted)?;
    Ok(Some(update))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct UpdateCheckCacheFile {
    version: u32,
    checked_unix_ms: i64,
    latest_tag: String,
}

fn update_cache_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join("update_check.json")
}

pub fn load_update_check_cache(state_dir: &Path) -> Result<Option<(i64, String)>, io::Error> {
    let path = update_cache_path(state_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let parsed: UpdateCheckCacheFile = match serde_json::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(None),
    };
    if parsed.latest_tag.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some((parsed.checked_unix_ms, parsed.latest_tag)))
}

pub fn save_update_check_cache(state_dir: &Path, latest_tag: &str) -> Result<(), io::Error> {
    fs::create_dir_all(state_dir)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let checked_unix_ms = i64::try_from(now).unwrap_or(i64::MAX);

    let file = UpdateCheckCacheFile {
        version: 1,
        checked_unix_ms,
        latest_tag: latest_tag.to_string(),
    };

    let path = update_cache_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(&file).unwrap_or_else(|_| "{}".to_string());
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn download_release_archive(tag: &str, version: &Version) -> Result<Vec<u8>, UpdateError> {
    let target = resolve_target_triple()?;
    let version = version.to_string();
    let artifact = format!("{BIN_NAME}-{version}-{target}.tar.gz");
    let base_url = format!("https://github.com/{REPO}/releases/download/{tag}");
    let archive_url = format!("{base_url}/{artifact}");
    let sha_url = format!("{archive_url}.sha256");

    let agent = make_agent(Duration::from_secs(20));

    let archive_bytes = http_get_bytes(&agent, &archive_url)?;
    let sha_bytes = http_get_bytes(&agent, &sha_url)?;
    let expected_sha = parse_sha256_file(&sha_bytes)?;
    let actual_sha = sha256_hex(&archive_bytes);
    if expected_sha != actual_sha {
        return Err(UpdateError::ShaMismatch { asset: artifact });
    }

    Ok(archive_bytes)
}

fn resolve_target_triple() -> Result<&'static str, UpdateError> {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            return Ok("aarch64-apple-darwin");
        }
        if cfg!(target_arch = "x86_64") {
            return Ok("x86_64-apple-darwin");
        }
        return Err(UpdateError::UnsupportedPlatform(
            "unsupported CPU architecture on macOS".to_string(),
        ));
    }

    if cfg!(target_os = "linux") {
        if cfg!(target_arch = "x86_64") {
            return Ok("x86_64-unknown-linux-gnu");
        }
        return Err(UpdateError::UnsupportedPlatform(
            "unsupported CPU architecture on Linux".to_string(),
        ));
    }

    Err(UpdateError::UnsupportedPlatform(
        "self-update is only supported on macOS/Linux (use Releases or your package manager)"
            .to_string(),
    ))
}

fn http_get_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, UpdateError> {
    let mut body = agent
        .get(url)
        .header(
            "User-Agent",
            &format!("{BIN_NAME}/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|error| UpdateError::Download(error.to_string()))?
        .into_body();

    body.read_to_vec()
        .map_err(|error| UpdateError::Download(error.to_string()))
}

fn make_agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build();
    config.into()
}

fn parse_sha256_file(bytes: &[u8]) -> Result<String, UpdateError> {
    let text = String::from_utf8_lossy(bytes);
    let mut iter = text.split_whitespace();
    let Some(hash) = iter.next() else {
        return Err(UpdateError::ParseSha("empty sha256 file".to_string()));
    };

    let normalized = hash.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(UpdateError::ParseSha(format!(
            "invalid sha256 value: {hash}"
        )));
    }
    Ok(normalized)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push(hex_char(b >> 4));
        out.push(hex_char(b & 0x0f));
    }
    out
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => '0',
    }
}

fn extract_binary_from_tar_gz(archive_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(archive_bytes));
    let mut archive = tar::Archive::new(gz);

    let entries = archive.entries().map_err(|error| error.to_string())?;
    for entry in entries {
        let mut entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().map_err(|error| error.to_string())?;
        if path.file_name().and_then(|name| name.to_str()) != Some(BIN_NAME) {
            continue;
        }

        let mut bytes: Vec<u8> = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.is_empty() {
            return Err("extracted binary is empty".to_string());
        }
        return Ok(bytes);
    }

    Err(format!("missing {BIN_NAME} in archive"))
}

fn install_binary(bytes: &[u8]) -> Result<(), UpdateError> {
    let current_exe =
        std::env::current_exe().map_err(|error| UpdateError::CurrentExe(error.to_string()))?;
    let Some(dir) = current_exe.parent() else {
        return Err(UpdateError::CurrentExe(format!(
            "executable has no parent directory: {}",
            current_exe.display()
        )));
    };

    let temp_name = format!(
        ".{BIN_NAME}.update.{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let temp_path = dir.join(temp_name);

    let mut file = fs::File::create(&temp_path).map_err(|error| UpdateError::Install {
        path: temp_path.display().to_string(),
        source: error,
    })?;
    file.write_all(bytes)
        .map_err(|error| UpdateError::Install {
            path: temp_path.display().to_string(),
            source: error,
        })?;
    file.flush().map_err(|error| UpdateError::Install {
        path: temp_path.display().to_string(),
        source: error,
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&temp_path, permissions).map_err(|error| UpdateError::Install {
            path: temp_path.display().to_string(),
            source: error,
        })?;
    }

    fs::rename(&temp_path, &current_exe).map_err(|error| UpdateError::Install {
        path: current_exe.display().to_string(),
        source: error,
    })?;

    Ok(())
}
