use super::*;
use crate::tests::TempDirectory;
use std::io::ErrorKind;

fn existing(path: &Path) -> SaveTarget {
    let target = inspect_save_target(path).unwrap();
    assert!(matches!(target, SaveTarget::Existing(_)));
    target
}

#[test]
fn creates_and_replaces_exact_utf8_bytes() {
    let temp = TempDirectory::new();
    let path = temp.0.join("中文 space.txt");
    for text in ["ASCII", "你好🙂 e\u{301}", "", "one\ntwo\n", "一\r\n二\r\n"] {
        let outcome = save_utf8(&path, text, &SaveTarget::New).unwrap();
        assert!(outcome.warning.is_none(), "{:?}", outcome.warning);
        let baseline = outcome.baseline.unwrap();
        assert_eq!(fs::read(&path).unwrap(), text.as_bytes());
        let outcome = save_utf8(&path, "replace", &SaveTarget::Existing(baseline)).unwrap();
        assert!(outcome.warning.is_none(), "{:?}", outcome.warning);
        assert!(outcome.baseline.is_some());
        assert_eq!(fs::read(&path).unwrap(), b"replace");
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
        fs::remove_file(&path).unwrap();
    }
}

#[test]
fn requires_observed_existing_destination_and_rechecks_before_commit() {
    let temp = TempDirectory::new();
    let path = temp.write(b"before");
    assert!(matches!(
        save_utf8(&path, "new", &SaveTarget::New),
        Err(SaveError::Conflict)
    ));
    let expected = existing(&path);
    assert!(matches!(
        save_with(&path, "new", &expected, |stage, _| {
            if stage == Stage::Commit {
                fs::write(&path, b"edited")?;
            }
            Ok(())
        }),
        Err(SaveError::Conflict)
    ));
    assert_eq!(fs::read(&path).unwrap(), b"edited");
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    let expected = existing(&path);
    let old = path.with_extension("old");
    fs::rename(&path, &old).unwrap();
    assert!(matches!(
        save_utf8(&path, "new", &expected),
        Err(SaveError::Conflict)
    ));
    fs::copy(&old, &path).unwrap();
    assert!(matches!(
        save_utf8(&path, "new", &expected),
        Err(SaveError::Conflict)
    ));
    assert_eq!(fs::read(&path).unwrap(), b"edited");
}

#[test]
fn late_creation_never_overwrites_an_unconfirmed_file() {
    let temp = TempDirectory::new();
    let path = temp.0.join("new");
    assert!(matches!(
        save_with(&path, "new", &SaveTarget::New, |stage, _| {
            if stage == Stage::Commit {
                fs::write(&path, b"racer")?;
            }
            Ok(())
        }),
        Err(SaveError::Conflict)
    ));
    assert_eq!(fs::read(path).unwrap(), b"racer");
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
}

#[test]
fn injected_failures_keep_original_and_clean_staging() {
    let temp = TempDirectory::new();
    let path = temp.write(b"original");
    let expected = existing(&path);
    for (failure, kind) in [
        (Stage::Create, ErrorKind::PermissionDenied),
        (Stage::Create, ErrorKind::AlreadyExists),
        (Stage::Write, ErrorKind::StorageFull),
        (Stage::Flush, ErrorKind::Other),
        (Stage::Sync, ErrorKind::Other),
        (Stage::Commit, ErrorKind::PermissionDenied),
    ] {
        let result = save_with(&path, "replacement", &expected, |stage, file| {
            if stage == failure {
                if stage == Stage::Write {
                    file.unwrap().write_all(b"partial")?;
                }
                return Err(kind.into());
            }
            Ok(())
        });
        assert!(
            matches!(result, Err(SaveError::Io(_, e)) if e.kind() == kind),
            "{failure:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }
}

#[test]
fn refuses_read_only_and_invalid_paths() {
    let temp = TempDirectory::new();
    let path = temp.write(b"original");
    let original = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let result = save_utf8(&path, "new", &existing(&path));
    fs::set_permissions(&path, original).unwrap();
    assert!(matches!(result, Err(SaveError::ReadOnly)));
    assert!(save_utf8(&temp.0, "new", &SaveTarget::New).is_err());
    assert!(
        save_utf8(
            &temp.0.join("missing").join("file"),
            "new",
            &SaveTarget::New
        )
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
}

#[test]
fn real_lock_keeps_original_and_cleans_staging() {
    let temp = TempDirectory::new();
    let path = temp.write(b"original");
    let expected = existing(&path);
    #[cfg(windows)]
    let lock = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap()
    };
    #[cfg(target_os = "linux")]
    let lock = {
        let file = File::open(&path).unwrap();
        platform::lock_target(&file).unwrap();
        file
    };
    #[cfg(any(windows, target_os = "linux"))]
    {
        assert!(save_utf8(&path, "new", &expected).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
        drop(lock);
        assert!(save_utf8(&path, "new", &expected).is_ok());
    }
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_in_destination_and_ancestors() {
    use std::os::unix::fs::symlink;
    let temp = TempDirectory::new();
    let path = temp.write(b"original");
    let link = temp.0.join("link");
    symlink(&path, &link).unwrap();
    assert!(matches!(
        inspect_save_target(&link),
        Err(SaveError::InvalidTarget)
    ));
    let folder_link = temp.0.join("folder");
    symlink(&temp.0, &folder_link).unwrap();
    assert!(matches!(
        inspect_save_target(&folder_link.join("new")),
        Err(SaveError::InvalidTarget)
    ));
    assert_eq!(fs::read(path).unwrap(), b"original");
}

#[cfg(target_os = "linux")]
#[test]
fn preserves_mode_and_rejects_unpreservable_attributes() {
    use std::os::{fd::AsRawFd, unix::fs::PermissionsExt};
    let temp = TempDirectory::new();
    let path = temp.write(b"original");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    save_utf8(&path, "new", &existing(&path)).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let file = File::open(&path).unwrap();
    // SAFETY: The descriptor, terminated name and one-byte value are live for this call.
    let result = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            c"user.rusteditor".as_ptr(),
            c"x".as_ptr().cast(),
            1,
            0,
        )
    };
    assert_eq!(result, 0);
    assert!(matches!(
        save_utf8(&path, "rejected", &existing(&path)),
        Err(SaveError::UnsupportedMetadata)
    ));
    assert_eq!(fs::read(&path).unwrap(), b"new");
}

#[cfg(target_os = "linux")]
#[test]
fn unwritable_directory_leaves_original_intact() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: geteuid has no arguments or memory requirements.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let temp = TempDirectory::new();
    let path = temp.write(b"original");
    let expected = existing(&path);
    let permissions = fs::metadata(&temp.0).unwrap().permissions();
    fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o500)).unwrap();
    let result = save_utf8(&path, "new", &expected);
    fs::set_permissions(&temp.0, permissions).unwrap();
    assert!(
        matches!(result, Err(SaveError::Io("create temporary file", e)) if e.kind() == ErrorKind::PermissionDenied)
    );
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn interrupted_save_child() {
    let Some(path) = std::env::var_os("RUSTEDITOR_TEST_INTERRUPT_SAVE").map(PathBuf::from) else {
        return;
    };
    let expected = existing(&path);
    let _ = save_with(&path, "staged", &expected, |stage, _| {
        if stage == Stage::Commit {
            std::process::exit(71);
        }
        Ok(())
    });
    panic!("child never reached the commit boundary");
}

#[test]
fn process_exit_before_commit_preserves_original() {
    let temp = TempDirectory::new();
    let path = temp.write(b"original");
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "save::tests::interrupted_save_child"])
        .env("RUSTEDITOR_TEST_INTERRUPT_SAVE", &path)
        .status()
        .unwrap();
    assert_eq!(result.code(), Some(71));
    assert_eq!(fs::read(&path).unwrap(), b"original");
    // An interrupted process cannot run the staging cleanup guard.
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 2);
}
