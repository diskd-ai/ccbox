use crate::types::{CcboxesFile, PairingRecord, TrustedDevicesFile};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct StorePaths {
    pub trusted_devices_path: PathBuf,
    pub ccboxes_path: PathBuf,
    pub pairings_dir: PathBuf,
}

pub fn make_store_paths(data_dir: &Path) -> StorePaths {
    StorePaths {
        trusted_devices_path: data_dir.join("trusted_devices.json"),
        ccboxes_path: data_dir.join("ccboxes.json"),
        pairings_dir: data_dir.join("pairings"),
    }
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("tmp");
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    fs::write(&tmp_path, format!("{text}\n"))?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let parsed = serde_json::from_str::<T>(&raw)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(parsed))
}

pub fn load_trusted_devices(paths: &StorePaths) -> io::Result<TrustedDevicesFile> {
    match read_json_file::<TrustedDevicesFile>(&paths.trusted_devices_path)? {
        Some(file) => Ok(file),
        None => Ok(TrustedDevicesFile::default()),
    }
}

pub fn save_trusted_devices(paths: &StorePaths, file: &TrustedDevicesFile) -> io::Result<()> {
    atomic_write_json(&paths.trusted_devices_path, file)
}

pub fn load_ccboxes(paths: &StorePaths) -> io::Result<CcboxesFile> {
    match read_json_file::<CcboxesFile>(&paths.ccboxes_path)? {
        Some(file) => Ok(file),
        None => Ok(CcboxesFile::default()),
    }
}

pub fn save_ccboxes(paths: &StorePaths, file: &CcboxesFile) -> io::Result<()> {
    atomic_write_json(&paths.ccboxes_path, file)
}

fn pairing_path_for_guid(paths: &StorePaths, guid: &str) -> io::Result<PathBuf> {
    if !crate::util::is_uuid(guid) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid guid"));
    }
    Ok(paths.pairings_dir.join(format!("{guid}.json")))
}

pub fn load_pairing(paths: &StorePaths, guid: &str) -> io::Result<Option<PairingRecord>> {
    let path = pairing_path_for_guid(paths, guid)?;
    read_json_file::<PairingRecord>(&path)
}

pub fn save_pairing(paths: &StorePaths, guid: &str, record: &PairingRecord) -> io::Result<()> {
    let path = pairing_path_for_guid(paths, guid)?;
    atomic_write_json(&path, record)
}

pub fn delete_pairing(paths: &StorePaths, guid: &str) -> io::Result<()> {
    let path = pairing_path_for_guid(paths, guid)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
