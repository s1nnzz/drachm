use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

pub(super) fn read_text(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

pub(super) fn create_text(path: &Path, text: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(text.as_bytes())
}

pub(super) fn write_text(path: &Path, text: &str) -> io::Result<()> {
    fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> io::Result<Self> {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);

            loop {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("drachm-fs-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Ok(Self(path)),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
            }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn exclusive_create_preserves_contents() -> io::Result<()> {
        let dir = TestDir::new()?;
        let path = dir.0.join("text");
        create_text(&path, "original")?;

        let error = create_text(&path, "replacement").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read_text(&path)?, "original");
        Ok(())
    }

    #[test]
    fn write_creates_and_replaces_utf8_text() -> io::Result<()> {
        let dir = TestDir::new()?;
        let path = dir.0.join("text");
        write_text(&path, "café au lait")?;
        assert_eq!(read_text(&path)?, "café au lait");

        write_text(&path, "茶")?;
        assert_eq!(read_text(&path)?, "茶");
        Ok(())
    }
}
