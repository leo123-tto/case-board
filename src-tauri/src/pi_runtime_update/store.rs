use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const FAILURE_THRESHOLD: u32 = 3;

#[derive(Debug, Clone)]
pub struct RuntimeStore {
    root: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeSelection {
    pub current: Option<String>,
    pub previous: Option<String>,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Runtime 版本号无效:{0}")]
    InvalidVersion(String),
    #[error("Runtime 版本尚未完整安装:{0}")]
    VersionMissing(String),
    #[error("Runtime 存储读写失败:{0}")]
    Io(#[from] std::io::Error),
    #[error("Runtime 存储状态损坏:{0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct CurrentFile {
    #[serde(default)]
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_version: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct BadVersionsFile {
    #[serde(default)]
    versions: BTreeSet<String>,
    #[serde(default)]
    failures: BTreeMap<String, u32>,
}

impl RuntimeStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn version_dir(&self, version: &str) -> Result<PathBuf, StoreError> {
        validate_version(version)?;
        Ok(self.root.join("versions").join(version))
    }

    pub fn version_binary(&self, version: &str) -> Result<PathBuf, StoreError> {
        Ok(self.version_dir(version)?.join(runtime_binary_name()))
    }

    pub fn selection(&self) -> Result<RuntimeSelection, StoreError> {
        let current = self.read_current()?;
        Ok(RuntimeSelection {
            current: nonempty(current.version),
            previous: current.previous_version.and_then(nonempty),
        })
    }

    pub fn activate(&self, version: &str) -> Result<(), StoreError> {
        let binary = self.version_binary(version)?;
        if !binary.is_file() {
            return Err(StoreError::VersionMissing(version.to_string()));
        }
        if self.is_bad(version)? {
            return Err(StoreError::VersionMissing(format!("{version}(已标记故障)")));
        }
        let current = self.read_current()?;
        let old_current = nonempty(current.version);
        let previous_version = if old_current.as_deref() == Some(version) {
            current.previous_version.and_then(nonempty)
        } else {
            old_current
        };
        self.write_current(&CurrentFile {
            version: version.to_string(),
            previous_version,
        })
    }

    pub fn rollback_to_bundled(&self) -> Result<(), StoreError> {
        let selection = self.selection()?;
        self.write_current(&CurrentFile {
            version: String::new(),
            previous_version: selection.current.or(selection.previous),
        })
    }

    /// Returns true once this failure caused the version to be quarantined.
    pub fn record_failure(&self, version: &str) -> Result<bool, StoreError> {
        validate_version(version)?;
        let mut bad = self.read_bad_versions()?;
        let count = {
            let count = bad.failures.entry(version.to_string()).or_default();
            *count = count.saturating_add(1);
            *count
        };
        let newly_bad = count >= FAILURE_THRESHOLD && bad.versions.insert(version.to_string());
        self.write_bad_versions(&bad)?;

        if count >= FAILURE_THRESHOLD {
            self.rollback_failed_current(version, &bad)?;
        }
        Ok(newly_bad)
    }

    pub fn record_success(&self, version: &str) -> Result<(), StoreError> {
        validate_version(version)?;
        let mut bad = self.read_bad_versions()?;
        if bad.failures.remove(version).is_some() {
            self.write_bad_versions(&bad)?;
        }
        Ok(())
    }

    pub fn is_bad(&self, version: &str) -> Result<bool, StoreError> {
        validate_version(version)?;
        Ok(self.read_bad_versions()?.versions.contains(version))
    }

    fn rollback_failed_current(
        &self,
        failed_version: &str,
        bad: &BadVersionsFile,
    ) -> Result<(), StoreError> {
        let selection = self.selection()?;
        if selection.current.as_deref() != Some(failed_version) {
            return Ok(());
        }
        let previous = selection.previous.filter(|version| {
            !bad.versions.contains(version)
                && self
                    .version_binary(version)
                    .map(|path| path.is_file())
                    .unwrap_or(false)
        });
        self.write_current(&CurrentFile {
            version: previous.unwrap_or_default(),
            previous_version: None,
        })
    }

    fn read_current(&self) -> Result<CurrentFile, StoreError> {
        read_json_or_default(&self.root.join("current.json"))
    }

    fn write_current(&self, value: &CurrentFile) -> Result<(), StoreError> {
        atomic_write_json(&self.root.join("current.json"), value)
    }

    fn read_bad_versions(&self) -> Result<BadVersionsFile, StoreError> {
        read_json_or_default(&self.root.join("bad-versions.json"))
    }

    fn write_bad_versions(&self, value: &BadVersionsFile) -> Result<(), StoreError> {
        atomic_write_json(&self.root.join("bad-versions.json"), value)
    }
}

fn runtime_binary_name() -> &'static str {
    if cfg!(windows) {
        "caseboard-pi-runtime.exe"
    } else {
        "caseboard-pi-runtime"
    }
}

fn validate_version(version: &str) -> Result<(), StoreError> {
    if version.is_empty()
        || version.contains(['/', '\\'])
        || version == "."
        || version == ".."
        || Version::parse(version).is_err()
    {
        return Err(StoreError::InvalidVersion(version.to_string()));
    }
    Ok(())
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn read_json_or_default<T>(path: &Path) -> Result<T, StoreError>
where
    T: for<'de> Deserialize<'de> + Default,
{
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(StoreError::from),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(StoreError::Io(error)),
    }
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::InvalidVersion("存储路径没有父目录".into()))?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| StoreError::Io(error.error))?;
    Ok(())
}
