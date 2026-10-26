//! The app's payload: the files it reads at run time, such as its Lynx
//! bundle, which go in the APK's `assets/` or the iOS app's bundle.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The files in a payload folder and its subfolders, sorted: each one's
/// path relative to the folder, with `/` between folders, and its
/// contents. Hidden files (`.DS_Store`, `.gitkeep`) are left out: they're
/// the file system's or git's rather than the app's, and aapt2 leaves them
/// out of an APK's assets too.
pub fn read(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    read_folder(root, "the payload folder")
}

/// The files in any folder that goes in an app, as `read` reads a
/// payload's, such as a native module's framework. `what` names the folder
/// in errors.
pub fn read_folder(root: &Path, what: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut folders = vec![(root.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = folders.pop() {
        let children = fs::read_dir(&folder)
            .and_then(Iterator::collect::<io::Result<Vec<_>>>)
            .with_context(|| format!("couldn't read {what} {}", folder.display()))?;
        for child in children {
            let name = child.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if child.path().is_dir() {
                folders.push((child.path(), format!("{prefix}{name}/")));
            } else {
                files.push((format!("{prefix}{name}"), child.path()));
            }
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|(name, path)| {
            let contents =
                fs::read(&path).with_context(|| format!("couldn't read {}", path.display()))?;
            Ok((name, contents))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::read;

    #[test]
    fn reads_every_file_but_hidden_ones() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("bundle/static")).unwrap();
        fs::write(dir.path().join("payload.txt"), "the app's payload").unwrap();
        fs::write(dir.path().join("bundle/main.lynx.bundle"), "bundle").unwrap();
        fs::write(dir.path().join("bundle/static/logo.png"), "logo").unwrap();
        fs::write(dir.path().join(".DS_Store"), "hidden").unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), "hidden").unwrap();

        let files = read(dir.path()).unwrap();

        assert_eq!(
            files,
            [
                ("bundle/main.lynx.bundle".to_owned(), b"bundle".to_vec()),
                ("bundle/static/logo.png".to_owned(), b"logo".to_vec()),
                ("payload.txt".to_owned(), b"the app's payload".to_vec()),
            ]
        );
    }

    #[test]
    fn reports_a_folder_it_cant_read() {
        let dir = tempfile::tempdir().unwrap();

        let error = read(&dir.path().join("missing")).unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("couldn't read the payload folder ")
        );
    }

    #[cfg(unix)]
    #[test]
    fn reports_a_file_it_cant_read() {
        let dir = tempfile::tempdir().unwrap();
        // A link to nothing looks like a file, but can't be read.
        std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("payload.txt"))
            .unwrap();

        let error = read(dir.path()).unwrap_err();

        assert!(error.to_string().starts_with("couldn't read "));
        assert!(error.to_string().ends_with("payload.txt"));
    }
}
