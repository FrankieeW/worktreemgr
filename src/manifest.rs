use std::{
    collections::BTreeMap,
    fs::File,
    io::Read as _,
    time::{Duration, UNIX_EPOCH},
};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use walkdir::{DirEntry, WalkDir};

use crate::error::WkError;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Manifest {
    pub entries: BTreeMap<Utf8PathBuf, ManifestEntry>,
}

impl Manifest {
    #[must_use]
    pub fn has_same_content_identity(&self, other: &Self) -> bool {
        if self.entries.len() != other.entries.len() {
            return false;
        }
        self.entries.iter().all(|(path, entry)| {
            other
                .entries
                .get(path)
                .is_some_and(|other_entry| entry.has_same_content_identity(other_entry))
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManifestEntry {
    pub kind: EntryKind,
    pub hash: Option<String>,
    pub target: Option<Utf8PathBuf>,
    pub executable: bool,
    pub size: u64,
    pub mtime: i128,
}

impl ManifestEntry {
    #[must_use]
    pub fn has_same_content_identity(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.hash == other.hash
            && self.target == other.target
            && self.executable == other.executable
    }
}

pub fn build_manifest(root: &Utf8Path) -> Result<Manifest, WkError> {
    let metadata = std::fs::symlink_metadata(root)?;
    let mut entries = BTreeMap::new();
    if metadata.is_dir() {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_descend)
        {
            let entry = entry?;
            let relative = relative_entry_path(root, &entry)?;
            if relative.as_str().is_empty() || is_reserved(relative) {
                continue;
            }
            let item = manifest_entry(
                Utf8Path::from_path(entry.path())
                    .ok_or_else(|| WkError::non_utf8_path(entry.path().display().to_string()))?,
            )?;
            entries.insert(relative.to_path_buf(), item);
        }
    } else {
        entries.insert(Utf8PathBuf::from(""), manifest_entry(root)?);
    }
    Ok(Manifest { entries })
}

fn manifest_entry(path: &Utf8Path) -> Result<ManifestEntry, WkError> {
    let metadata = std::fs::symlink_metadata(path)?;
    let kind = entry_kind(path, &metadata)?;
    let hash = if kind == EntryKind::File {
        Some(hash_file(path)?)
    } else {
        None
    };
    let target = if kind == EntryKind::Symlink {
        Some(read_link_utf8(path)?)
    } else {
        None
    };
    Ok(ManifestEntry {
        kind,
        hash,
        target,
        executable: is_executable(&metadata),
        size: metadata.len(),
        mtime: modified_nanos(&metadata)?,
    })
}

fn entry_kind(path: &Utf8Path, metadata: &std::fs::Metadata) -> Result<EntryKind, WkError> {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Ok(EntryKind::Symlink);
    }
    if file_type.is_dir() {
        return Ok(EntryKind::Directory);
    }
    if file_type.is_file() {
        return Ok(EntryKind::File);
    }
    Err(WkError::message(format!(
        "unsupported filesystem entry in manifest: {path}"
    )))
}

fn hash_file(path: &Utf8Path) -> Result<String, WkError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn read_link_utf8(path: &Utf8Path) -> Result<Utf8PathBuf, WkError> {
    let target = std::fs::read_link(path)?;
    Utf8PathBuf::from_path_buf(target)
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

fn modified_nanos(metadata: &std::fs::Metadata) -> Result<i128, WkError> {
    let modified = metadata.modified()?;
    Ok(match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration_nanos(duration),
        Err(error) => -duration_nanos(error.duration()),
    })
}

fn duration_nanos(duration: Duration) -> i128 {
    i128::from(duration.as_secs()) * 1_000_000_000_i128 + i128::from(duration.subsec_nanos())
}

fn should_descend(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    name != ".git" && name != ".wk"
}

fn relative_entry_path<'a>(root: &Utf8Path, entry: &'a DirEntry) -> Result<&'a Utf8Path, WkError> {
    let path = Utf8Path::from_path(entry.path())
        .ok_or_else(|| WkError::non_utf8_path(entry.path().display().to_string()))?;
    path.strip_prefix(root)
        .map_err(|error| WkError::message(error.to_string()))
}

fn is_reserved(path: &Utf8Path) -> bool {
    path.as_str()
        .split('/')
        .any(|segment| segment == ".git" || segment == ".wk")
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}
