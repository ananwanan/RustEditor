use crate::{MAX_DOCUMENT_BYTES, Metadata};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

mod platform;

#[derive(Debug)]
pub enum SaveError {
    InvalidTarget,
    Conflict,
    ReadOnly,
    UnsupportedMetadata,
    TooLarge,
    Io(&'static str, io::Error),
    OriginalRetained { backup: PathBuf, error: io::Error },
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTarget => {
                f.write_str("expected a regular file path without links or reparse points")
            }
            Self::Conflict => f.write_str("destination changed on disk. Save As to another path"),
            Self::ReadOnly => f.write_str("destination is read-only"),
            Self::UnsupportedMetadata => {
                f.write_str("cannot safely preserve the destination's metadata")
            }
            Self::TooLarge => write!(f, "file exceeds the {MAX_DOCUMENT_BYTES}-byte limit"),
            Self::Io(stage, error) => write!(f, "{stage}: {error}"),
            Self::OriginalRetained { backup, error } => write!(
                f,
                "replacement failed: {error}. Original retained at {}",
                backup.display()
            ),
        }
    }
}

impl std::error::Error for SaveError {}

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    identity: platform::Identity,
    modified: SystemTime,
    length: u64,
    attributes: u32,
    #[cfg(unix)]
    change_time: (i64, i64),
}

impl FileStamp {
    fn read(file: &File, metadata: &Metadata) -> io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            identity: platform::identity(file, metadata)?,
            modified: metadata.modified()?,
            length: metadata.len(),
            attributes: platform::attributes(metadata),
            #[cfg(unix)]
            change_time: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

/// Exact bounded disk contents and file identity observed through one open handle.
#[derive(Debug, PartialEq, Eq)]
pub struct DiskBaseline {
    stamp: FileStamp,
    bytes: Vec<u8>,
}

impl DiskBaseline {
    pub(crate) fn from_open(file: &File, metadata: &Metadata, bytes: &[u8]) -> io::Result<Self> {
        let stamp = FileStamp::read(file, metadata)?;
        Self::from_read(file, stamp, bytes.to_vec())
    }

    fn from_read(file: &File, stamp: FileStamp, bytes: Vec<u8>) -> io::Result<Self> {
        if FileStamp::read(file, &file.metadata()?)? != stamp || bytes.len() as u64 != stamp.length
        {
            return Err(io::Error::other("file changed while being read"));
        }
        Ok(Self { stamp, bytes })
    }
}

/// An existing destination must be inspected and explicitly confirmed before replacement.
#[derive(Debug)]
pub enum SaveTarget {
    New,
    Existing(DiskBaseline),
}

pub struct SaveOutcome {
    /// A committed file may be unavailable for reinspection. Require Save As in that case.
    pub baseline: Option<DiskBaseline>,
    pub warning: Option<String>,
}

fn valid_target(path: &Path) -> Result<PathBuf, SaveError> {
    // Reject parent traversal rather than resolving it across a possible directory link.
    if path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(SaveError::InvalidTarget);
    }
    let path = std::path::absolute(path).map_err(|e| SaveError::Io("resolve destination", e))?;
    let name = path.file_name().ok_or(SaveError::InvalidTarget)?;
    platform::validate_name(&path, name)?;
    let parent = path.parent().ok_or(SaveError::InvalidTarget)?;
    let ancestors: Vec<_> = parent.ancestors().collect();
    for ancestor in ancestors.into_iter().rev().filter(|p| p.has_root()) {
        // The root is included so a mounted/reparse directory cannot redirect this write.
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|e| SaveError::Io("inspect destination directory", e))?;
        if !metadata.is_dir() || platform::is_link(&metadata) {
            return Err(SaveError::InvalidTarget);
        }
    }
    Ok(path)
}

fn open_target(path: &Path) -> Result<Option<File>, SaveError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SaveError::Io("inspect destination", error)),
    };
    if !metadata.is_file() || platform::is_link(&metadata) {
        return Err(SaveError::InvalidTarget);
    }
    let file = platform::open_target(path).map_err(|e| SaveError::Io("open destination", e))?;
    Ok(Some(file))
}

fn inspect_file(file: &File) -> Result<DiskBaseline, SaveError> {
    let metadata = file
        .metadata()
        .map_err(|e| SaveError::Io("inspect destination", e))?;
    if !metadata.is_file() || platform::is_link(&metadata) {
        return Err(SaveError::InvalidTarget);
    }
    if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(SaveError::TooLarge);
    }
    let stamp =
        FileStamp::read(file, &metadata).map_err(|e| SaveError::Io("identify destination", e))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| SaveError::Io("read destination", e))?;
    DiskBaseline::from_read(file, stamp, bytes).map_err(|e| SaveError::Io("check destination", e))
}

pub fn inspect_save_target(path: &Path) -> Result<SaveTarget, SaveError> {
    let path = valid_target(path)?;
    match open_target(&path)? {
        Some(file) => Ok(SaveTarget::Existing(inspect_file(&file)?)),
        None => Ok(SaveTarget::New),
    }
}

fn check_target(path: &Path, expected: &SaveTarget) -> Result<Option<File>, SaveError> {
    match (expected, open_target(path)?) {
        (SaveTarget::New, None) => Ok(None),
        (SaveTarget::Existing(expected), Some(file)) => {
            platform::lock_target(&file)?;
            let metadata = file
                .metadata()
                .map_err(|e| SaveError::Io("inspect permissions", e))?;
            if metadata.permissions().readonly() {
                return Err(SaveError::ReadOnly);
            }
            if inspect_file(&file)? != *expected {
                return Err(SaveError::Conflict);
            }
            Ok(Some(file))
        }
        _ => Err(SaveError::Conflict),
    }
}

struct Temporary {
    path: PathBuf,
    published: bool,
}

impl Temporary {
    fn create(parent: &Path) -> io::Result<(Self, File)> {
        for _ in 0..128 {
            let path = unique_path(parent);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok((
                        Self {
                            path,
                            published: false,
                        },
                        file,
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "temporary names exhausted",
        ))
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn unique_path(parent: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    parent.join(format!(
        ".rusteditor-save-{}-{:x}-{:x}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Create,
    Write,
    Flush,
    Sync,
    Commit,
}

pub fn save_utf8(
    path: &Path,
    content: &str,
    expected: &SaveTarget,
) -> Result<SaveOutcome, SaveError> {
    save_with(path, content, expected, |_, _| Ok(()))
}

fn save_with(
    path: &Path,
    content: &str,
    expected: &SaveTarget,
    mut before: impl FnMut(Stage, Option<&mut File>) -> io::Result<()>,
) -> Result<SaveOutcome, SaveError> {
    if content.len() > MAX_DOCUMENT_BYTES {
        return Err(SaveError::TooLarge);
    }
    let path = valid_target(path)?;
    let parent = path.parent().expect("validated destination");
    let original = check_target(&path, expected)?;
    before(Stage::Create, None).map_err(|e| SaveError::Io("create temporary file", e))?;
    let (mut temporary, mut file) =
        Temporary::create(parent).map_err(|e| SaveError::Io("create temporary file", e))?;
    #[cfg(windows)]
    if let Some(original) = original.as_ref() {
        platform::prepare_replacement(&path, original, &file)?;
    }
    before(Stage::Write, Some(&mut file))
        .and_then(|()| file.write_all(content.as_bytes()))
        .map_err(|e| SaveError::Io("write temporary file", e))?;
    before(Stage::Flush, Some(&mut file))
        .and_then(|()| file.flush())
        .map_err(|e| SaveError::Io("flush temporary file", e))?;
    #[cfg(unix)]
    if let Some(original) = original.as_ref() {
        platform::prepare_replacement(&path, original, &file)?;
    }
    before(Stage::Sync, Some(&mut file))
        .and_then(|()| file.sync_all())
        .map_err(|e| SaveError::Io("sync temporary file", e))?;
    let identity = file
        .metadata()
        .and_then(|m| platform::identity(&file, &m))
        .map_err(|e| SaveError::Io("identify temporary file", e))?;
    drop(original);

    before(Stage::Commit, None).map_err(|e| SaveError::Io("commit save", e))?;
    // Recheck immediately before the platform operation. Existing-file replacement
    // still has a race against changes between this check and the filesystem commit.
    valid_target(&path)?;
    let current = check_target(&path, expected)?;
    let replace = current.is_some();
    #[cfg(windows)]
    if let Some(destination) = current.as_ref() {
        platform::prepare_replacement(&path, destination, &file)?;
    }
    drop(file);
    let mut warning = platform::commit(&temporary.path, &path, replace)?;
    temporary.published = true;
    if let Err(error) = platform::sync_parent(parent) {
        add_warning(
            &mut warning,
            format!("could not sync destination directory: {error}"),
        );
    }
    let baseline = match inspect_save_target(&path) {
        Ok(SaveTarget::Existing(baseline))
            if baseline.stamp.identity == identity && baseline.bytes == content.as_bytes() =>
        {
            Some(baseline)
        }
        _ => {
            add_warning(
                &mut warning,
                "could not verify committed destination. Use Save As".into(),
            );
            None
        }
    };
    Ok(SaveOutcome { baseline, warning })
}

fn add_warning(warning: &mut Option<String>, message: String) {
    let warning = warning.get_or_insert_with(String::new);
    if !warning.is_empty() {
        warning.push_str(". ");
    }
    warning.push_str(&message);
}

#[cfg(test)]
mod tests;
