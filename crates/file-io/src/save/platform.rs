use super::SaveError;
use std::{ffi::OsStr, fs::File, io, path::Path};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::*;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs::{self, Metadata, OpenOptions},
        os::unix::fs::{MetadataExt, PermissionsExt},
    };

    #[derive(Debug, PartialEq, Eq)]
    pub(in crate::save) struct Identity(u64, u64);

    pub(in crate::save) fn identity(_: &File, metadata: &Metadata) -> io::Result<Identity> {
        Ok(Identity(metadata.dev(), metadata.ino()))
    }

    pub(in crate::save) fn attributes(metadata: &Metadata) -> u32 {
        metadata.mode()
    }

    pub(in crate::save) fn is_link(metadata: &Metadata) -> bool {
        metadata.file_type().is_symlink()
    }

    pub(in crate::save) fn validate_name(_: &Path, _: &OsStr) -> Result<(), SaveError> {
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(in crate::save) fn open_target(path: &Path) -> io::Result<File> {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
    }

    #[cfg(not(target_os = "linux"))]
    pub(in crate::save) fn open_target(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    #[cfg(target_os = "linux")]
    fn has_extended_attributes(file: &File) -> io::Result<bool> {
        use std::os::fd::AsRawFd;
        // SAFETY: A live descriptor and a zero-size query require no output buffer.
        let count = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(count != 0)
    }

    pub(in crate::save) fn prepare_replacement(
        path: &Path,
        original: &File,
        temporary: &File,
    ) -> Result<(), SaveError> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let original_metadata = original
                .metadata()
                .map_err(|e| SaveError::Io("inspect permissions", e))?;
            let temporary_metadata = temporary
                .metadata()
                .map_err(|e| SaveError::Io("inspect temporary permissions", e))?;
            if original_metadata.uid() != temporary_metadata.uid()
                || original_metadata.gid() != temporary_metadata.gid()
                || original_metadata.nlink() != 1
                || original_metadata.mode() & 0o7000 != 0
                || has_extended_attributes(original)
                    .map_err(|e| SaveError::Io("inspect destination ACL and attributes", e))?
                || has_extended_attributes(temporary)
                    .map_err(|e| SaveError::Io("inspect temporary ACL and attributes", e))?
            {
                return Err(SaveError::UnsupportedMetadata);
            }
            OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .map_err(|e| SaveError::Io("check destination write access", e))?;
            temporary
                .set_permissions(fs::Permissions::from_mode(original_metadata.mode() & 0o777))
                .map_err(|e| SaveError::Io("preserve destination permissions", e))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, original, temporary);
            Err(SaveError::UnsupportedMetadata)
        }
    }

    pub(in crate::save) fn lock_target(file: &File) -> Result<(), SaveError> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: The file owns the descriptor for the lifetime of the lock.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(SaveError::Io(
                    "lock destination",
                    io::Error::last_os_error(),
                ));
            }
        }
        let _ = file;
        Ok(())
    }

    pub(in crate::save) fn commit(
        temp: &Path,
        path: &Path,
        replace: bool,
    ) -> Result<Option<String>, SaveError> {
        // Creating a hard link publishes the already-synced inode without clobbering
        // a file created after the last check. The staging name is cleaned up below.
        if replace {
            fs::rename(temp, path)
        } else {
            fs::hard_link(temp, path)
        }
        .map_err(|e| SaveError::Io("commit save", e))?;
        if !replace && let Err(error) = fs::remove_file(temp) {
            return Ok(Some(format!("could not remove staging name: {error}")));
        }
        Ok(None)
    }

    pub(in crate::save) fn sync_parent(parent: &Path) -> io::Result<()> {
        File::open(parent)?.sync_all()
    }
}

#[cfg(unix)]
pub(super) use unix::*;
