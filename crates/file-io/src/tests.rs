use super::*;
use std::{
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

pub(crate) struct TempDirectory(pub(crate) PathBuf);

impl TempDirectory {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rusteditor-open-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("fixture.txt");
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn preserves_valid_utf8_bytes() {
    let temp = TempDirectory::new();
    for text in [
        "",
        "plain ASCII",
        "你好",
        "🙂 e\u{301}",
        "a\nb\n",
        "中\r\n🙂\r\n",
    ] {
        let path = temp.write(text.as_bytes());
        assert_eq!(open_utf8(&path).unwrap().as_bytes(), text.as_bytes());
        assert_eq!(fs::read(path).unwrap(), text.as_bytes());
    }
}

#[test]
fn rejects_invalid_utf8() {
    let temp = TempDirectory::new();
    assert!(matches!(
        open_utf8(&temp.write(b"a\xffb")),
        Err(OpenError::InvalidUtf8(_))
    ));
}

#[test]
fn rejects_unsupported_boms() {
    let temp = TempDirectory::new();
    for bytes in [
        b"\xef\xbb\xbftext".as_slice(),
        b"\xff\xfe",
        b"\xfe\xff",
        b"\xff\xfe\x00\x00",
        b"\x00\x00\xfe\xff",
    ] {
        assert!(matches!(
            open_utf8(&temp.write(bytes)),
            Err(OpenError::UnsupportedBom)
        ));
    }
}

#[test]
fn distinguishes_missing_file_and_directory() {
    let temp = TempDirectory::new();
    assert!(matches!(
        open_utf8(&temp.0.join("missing")),
        Err(OpenError::NotFound)
    ));
    assert!(matches!(open_utf8(&temp.0), Err(OpenError::IsDirectory)));
}

#[test]
fn opens_read_only_file() {
    let temp = TempDirectory::new();
    let path = temp.write(b"read only");
    let original = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let result = open_utf8(&path);
    let permissions = fs::metadata(&path).unwrap().permissions();
    fs::set_permissions(&path, original).unwrap();
    assert_eq!(result.unwrap(), "read only");
    assert!(permissions.readonly());
    assert_eq!(fs::read(path).unwrap(), b"read only");
}

#[test]
fn enforces_exact_document_size_limit() {
    let temp = TempDirectory::new();
    let bytes = "🙂".repeat(MAX_DOCUMENT_BYTES / "🙂".len());
    assert_eq!(bytes.len(), MAX_DOCUMENT_BYTES);
    assert_eq!(open_utf8(&temp.write(bytes.as_bytes())).unwrap(), bytes);
    assert!(matches!(
        open_utf8(&temp.write(&vec![b'a'; MAX_DOCUMENT_BYTES + 1])),
        Err(OpenError::TooLarge)
    ));
}

#[test]
fn stale_metadata_cannot_bypass_read_limit() {
    let mut growing_file = Cursor::new(vec![b'a'; MAX_DOCUMENT_BYTES + 1024]);
    assert!(matches!(
        read_utf8(&mut growing_file, 1),
        Err(OpenError::TooLarge)
    ));
    assert_eq!(growing_file.position(), (MAX_DOCUMENT_BYTES + 1) as u64);
    assert_eq!(read_utf8(b"short".as_slice(), 12).unwrap(), "short");
}

#[test]
fn distinguishes_permission_and_read_failures() {
    struct FailingReader(io::ErrorKind);
    impl Read for FailingReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(self.0.into())
        }
    }
    assert!(matches!(
        read_utf8(FailingReader(io::ErrorKind::PermissionDenied), 0),
        Err(OpenError::PermissionDenied)
    ));
    assert!(matches!(
        read_utf8(FailingReader(io::ErrorKind::Other), 0),
        Err(OpenError::Read(_))
    ));
}
