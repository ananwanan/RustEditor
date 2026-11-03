use super::*;
use std::{
    fs::{self, Metadata, OpenOptions},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    ptr,
};
use windows_sys::Win32::{
    Foundation::ERROR_INSUFFICIENT_BUFFER,
    Security::{
        DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, GetKernelObjectSecurity,
        LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_COMPRESSED,
        FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_NOT_CONTENT_INDEXED, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_OPEN_REPARSE_POINT, GetFileInformationByHandle, MOVEFILE_WRITE_THROUGH,
        MoveFileExW, ReplaceFileW,
    },
};

#[derive(Debug, PartialEq, Eq)]
pub(in crate::save) struct Identity(u32, u32, u32);

fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: The file handle is live and info is a writable output structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

pub(in crate::save) fn identity(file: &File, _: &Metadata) -> io::Result<Identity> {
    let info = information(file)?;
    Ok(Identity(
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}

pub(in crate::save) fn attributes(metadata: &Metadata) -> u32 {
    metadata.file_attributes()
}

pub(in crate::save) fn is_link(metadata: &Metadata) -> bool {
    attributes(metadata) & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

pub(in crate::save) fn validate_name(path: &Path, name: &OsStr) -> Result<(), SaveError> {
    use std::path::{Component, Prefix};
    if !matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        || name.encode_wide().any(|unit| matches!(unit, 0 | 58))
        || name
            .encode_wide()
            .last()
            .is_some_and(|unit| matches!(unit, 32 | 46))
    {
        return Err(SaveError::InvalidTarget);
    }
    Ok(())
}

pub(in crate::save) fn open_target(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn security(file: &File) -> io::Result<Vec<u32>> {
    let flags = OWNER_SECURITY_INFORMATION
        | GROUP_SECURITY_INFORMATION
        | DACL_SECURITY_INFORMATION
        | LABEL_SECURITY_INFORMATION;
    let mut needed = 0;
    // SAFETY: A null output with zero length queries the required buffer size.
    unsafe {
        GetKernelObjectSecurity(file.as_raw_handle(), flags, ptr::null_mut(), 0, &mut needed)
    };
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) || needed == 0 {
        return Err(error);
    }
    let mut buffer = vec![0u32; (needed as usize).div_ceil(4)];
    // SAFETY: The buffer is aligned and at least needed bytes long.
    if unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            flags,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(buffer)
}

pub(in crate::save) fn prepare_replacement(
    path: &Path,
    original: &File,
    temporary: &File,
) -> Result<(), SaveError> {
    let info =
        information(original).map_err(|e| SaveError::Io("inspect destination attributes", e))?;
    let staging =
        information(temporary).map_err(|e| SaveError::Io("inspect temporary attributes", e))?;
    let security = |file| security(file).map_err(|e| SaveError::Io("inspect file security", e));
    // Staged contents must have the same access protection before the first byte is written.
    if info.nNumberOfLinks != 1
        || info.dwFileAttributes
            & !(FILE_ATTRIBUTE_ARCHIVE
                | FILE_ATTRIBUTE_COMPRESSED
                | FILE_ATTRIBUTE_NORMAL
                | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED)
            != 0
        || (info.dwFileAttributes ^ staging.dwFileAttributes) & FILE_ATTRIBUTE_NOT_CONTENT_INDEXED
            != 0
        || security(original)? != security(temporary)?
    {
        return Err(SaveError::UnsupportedMetadata);
    }
    OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| SaveError::Io("check destination write access", e))?;
    Ok(())
}

pub(in crate::save) fn lock_target(_: &File) -> Result<(), SaveError> {
    // Windows replacement honors the sharing mode of other open handles.
    Ok(())
}

fn wide(path: &Path) -> Vec<u16> {
    let mut name: Vec<u16> = Vec::new();
    if matches!(path.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::Disk(_)))
    {
        name.extend(r"\\?\".encode_utf16());
    }
    name.extend(path.as_os_str().encode_wide().map(|unit| {
        if unit == u16::from(b'/') {
            u16::from(b'\\')
        } else {
            unit
        }
    }));
    name.push(0);
    name
}

fn move_new(source: &Path, destination: &Path) -> io::Result<()> {
    let source = wide(source);
    let destination = wide(destination);
    // SAFETY: Both names are terminated and live for the call. Copy and overwrite are disabled.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(in crate::save) fn commit(
    temp: &Path,
    path: &Path,
    replace: bool,
) -> Result<Option<String>, SaveError> {
    if !replace {
        move_new(temp, path).map_err(|e| SaveError::Io("publish new file", e))?;
        return Ok(None);
    }

    // Reserve an empty same-volume directory for the original. ReplaceFileW can
    // fail after moving the original, so the backup path must always be supplied.
    let directory = crate::save::unique_path(path.parent().expect("validated destination"));
    fs::create_dir(&directory).map_err(|e| SaveError::Io("reserve replacement backup", e))?;
    let backup = directory.join("original");
    let source = wide(temp);
    let destination = wide(path);
    let backup_name = wide(&backup);
    // SAFETY: The paths are terminated and live. Reserved arguments are null.
    // No flags suppress security or attribute merge errors.
    let result = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            source.as_ptr(),
            backup_name.as_ptr(),
            0,
            ptr::null(),
            ptr::null(),
        )
    };
    if result == 0 {
        let error = io::Error::last_os_error();
        return Err(recover_original(path, backup, error));
    }

    if let Err(error) = fs::remove_file(&backup).and_then(|()| fs::remove_dir(&directory)) {
        return Ok(Some(format!(
            "could not remove replacement backup {}: {error}",
            directory.display()
        )));
    }
    Ok(None)
}

fn recover_original(path: &Path, backup: std::path::PathBuf, error: io::Error) -> SaveError {
    if !matches!(fs::symlink_metadata(&backup), Err(e) if e.kind() == io::ErrorKind::NotFound)
        && move_new(&backup, path).is_err()
    {
        return SaveError::OriginalRetained { backup, error };
    }
    let _ = fs::remove_dir(backup.parent().expect("reserved backup directory"));
    SaveError::Io("replace destination", error)
}

pub(in crate::save) fn sync_parent(_: &Path) -> io::Result<()> {
    // Windows does not expose the Unix directory fsync contract.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SaveTarget, inspect_save_target, save_utf8, tests::TempDirectory};
    use windows_sys::Win32::Security::{PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW};

    #[test]
    fn replacement_preserves_named_streams() {
        let temp = TempDirectory::new();
        let path = temp.write(b"original");
        let stream = path.with_file_name("fixture.txt:rusteditor-test");
        fs::write(&stream, b"stream data").unwrap();
        let result = save_utf8(&path, "replacement", &inspect_save_target(&path).unwrap()).unwrap();
        assert!(result.warning.is_none());
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        assert_eq!(fs::read(&stream).unwrap(), b"stream data");
        assert!(matches!(
            inspect_save_target(&stream),
            Err(SaveError::InvalidTarget)
        ));
    }

    #[test]
    fn refuses_an_acl_that_cannot_be_matched_by_the_staging_file() {
        let temp = TempDirectory::new();
        let path = temp.write(b"original");
        let mut descriptor = security(&File::open(&path).unwrap()).unwrap();
        let name = wide(&path);
        // SAFETY: The descriptor is aligned and valid, and the path is terminated.
        let result = unsafe {
            SetFileSecurityW(
                name.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor.as_mut_ptr().cast(),
            )
        };
        assert_ne!(result, 0, "{}", io::Error::last_os_error());
        let expected = inspect_save_target(&path).unwrap();
        assert!(matches!(expected, SaveTarget::Existing(_)));
        assert!(matches!(
            save_utf8(&path, "replacement", &expected),
            Err(SaveError::UnsupportedMetadata)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }

    #[test]
    fn recovery_restores_vacant_path_and_retains_original_on_collision() {
        let temp = TempDirectory::new();
        let path = temp.0.join("destination");
        let directory = temp.0.join("backup");
        let backup = directory.join("original");
        fs::create_dir(&directory).unwrap();
        fs::write(&backup, b"original").unwrap();
        fs::write(&path, b"concurrent").unwrap();
        let error = || io::Error::other("simulated partial ReplaceFileW failure");
        assert!(matches!(
            recover_original(&path, backup.clone(), error()),
            SaveError::OriginalRetained { backup: retained, .. } if retained == backup
        ));
        assert_eq!(fs::read(&path).unwrap(), b"concurrent");
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        fs::remove_file(&path).unwrap();
        assert!(matches!(
            recover_original(&path, backup, error()),
            SaveError::Io(_, _)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert!(!directory.exists());
    }
}
