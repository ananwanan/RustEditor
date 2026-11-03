#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod save;
pub use save::{DiskBaseline, SaveError, SaveOutcome, SaveTarget, inspect_save_target, save_utf8};

use rusteditor_editor_core::MAX_DOCUMENT_BYTES;
use std::{
    fmt,
    fs::{self, File, Metadata},
    io::{self, Read},
    path::Path,
    str::Utf8Error,
};

#[derive(Debug)]
pub enum OpenError {
    NotFound,
    PermissionDenied,
    IsDirectory,
    NotRegularFile,
    Open(io::Error),
    Read(io::Error),
    TooLarge,
    UnsupportedBom,
    InvalidUtf8(Utf8Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("file not found"),
            Self::PermissionDenied => f.write_str("permission denied"),
            Self::IsDirectory => f.write_str("path is a directory"),
            Self::NotRegularFile => f.write_str("path is not a regular file"),
            Self::Open(error) => write!(f, "could not open file: {error}"),
            Self::Read(error) => write!(f, "could not read file: {error}"),
            Self::TooLarge => write!(f, "file exceeds the {MAX_DOCUMENT_BYTES}-byte limit"),
            Self::UnsupportedBom => {
                f.write_str("unsupported byte order mark; expected UTF-8 without BOM")
            }
            Self::InvalidUtf8(error) => write!(f, "invalid UTF-8: {error}"),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open(error) | Self::Read(error) => Some(error),
            Self::InvalidUtf8(error) => Some(error),
            _ => None,
        }
    }
}

/// Reads one regular file as strict UTF-8 without a BOM, preserving its line endings.
pub fn open_utf8(path: &Path) -> Result<String, OpenError> {
    open_utf8_impl(path, false).map(|(text, _)| text)
}

/// Also capture the disk state from the handle that supplied the editor contents.
pub fn open_utf8_for_editing(path: &Path) -> Result<(String, DiskBaseline), OpenError> {
    let (text, baseline) = open_utf8_impl(path, true)?;
    Ok((text, baseline.expect("baseline requested")))
}

fn open_utf8_impl(path: &Path, track: bool) -> Result<(String, Option<DiskBaseline>), OpenError> {
    // Windows can report access denied when opening a directory as a file.
    check_file_type(&fs::metadata(path).map_err(open_error)?)?;
    let file = File::open(path).map_err(open_error)?;
    let metadata = file.metadata().map_err(read_error)?;
    check_file_type(&metadata)?;
    if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(OpenError::TooLarge);
    }
    let text = read_utf8(&file, metadata.len() as usize)?;
    let baseline = track
        .then(|| DiskBaseline::from_open(&file, &metadata, text.as_bytes()))
        .transpose()
        .map_err(read_error)?;
    Ok((text, baseline))
}

fn check_file_type(metadata: &Metadata) -> Result<(), OpenError> {
    if metadata.is_dir() {
        Err(OpenError::IsDirectory)
    } else if !metadata.is_file() {
        Err(OpenError::NotRegularFile)
    } else {
        Ok(())
    }
}

fn open_error(error: io::Error) -> OpenError {
    match error.kind() {
        io::ErrorKind::NotFound => OpenError::NotFound,
        io::ErrorKind::PermissionDenied => OpenError::PermissionDenied,
        io::ErrorKind::IsADirectory => OpenError::IsDirectory,
        _ => OpenError::Open(error),
    }
}

fn read_error(error: io::Error) -> OpenError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => OpenError::PermissionDenied,
        _ => OpenError::Read(error),
    }
}

fn read_utf8(reader: impl Read, size_hint: usize) -> Result<String, OpenError> {
    let mut bytes = Vec::with_capacity(size_hint + 1);
    reader
        .take((MAX_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(OpenError::TooLarge);
    }
    if [
        b"\xef\xbb\xbf".as_slice(),
        b"\xff\xfe",
        b"\xfe\xff",
        b"\x00\x00\xfe\xff",
    ]
    .iter()
    .any(|bom| bytes.starts_with(bom))
    {
        return Err(OpenError::UnsupportedBom);
    }
    String::from_utf8(bytes).map_err(|error| OpenError::InvalidUtf8(error.utf8_error()))
}

#[cfg(test)]
mod tests;
