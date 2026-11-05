//! The dock's places on the right: the Downloads folder and the Trash.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use std::{env, fs, io};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Downloads,
    Trash,
}

impl Place {
    /// In dock order.
    pub const ALL: [Place; 2] = [Place::Downloads, Place::Trash];

    pub fn name(self) -> &'static str {
        match self {
            Place::Downloads => "Downloads",
            Place::Trash => "Trash",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Download {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// The Downloads folder from the XDG user dirs, or `~/Downloads`.
pub fn downloads_dir() -> PathBuf {
    user_dir("XDG_DOWNLOAD_DIR").unwrap_or_else(|| home().join("Downloads"))
}

/// One of the folders in XDG's `user-dirs.dirs`, like `XDG_DESKTOP_DIR`.
pub fn user_dir(key: &str) -> Option<PathBuf> {
    let text = fs::read_to_string(config_home().join("user-dirs.dirs")).ok()?;
    for line in text.lines() {
        let Some(value) = line
            .trim()
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
        else {
            continue;
        };
        let value = value.trim_matches('"');
        let path = match value.strip_prefix("$HOME") {
            Some(rest) => home().join(rest.trim_start_matches('/')),
            None => PathBuf::from(value),
        };
        if path.is_absolute() {
            return Some(path);
        }
    }
    None
}

/// Moves a file or folder to the trash, per the freedesktop trash spec, so it can be restored.
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    let trash = trash_dir();
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("it has no name"))?
        .to_string_lossy()
        .into_owned();
    let date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
    // The info file claims the name, atomically, before the file itself goes there.
    for number in 1u32.. {
        let unique = if number == 1 {
            name.clone()
        } else {
            format!("{name}.{number}")
        };
        let info_path = info.join(format!("{unique}.trashinfo"));
        let mut claim = match fs::File::create_new(&info_path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        let original = file_uri(path);
        let encoded = original.strip_prefix("file://").unwrap_or(&original);
        claim
            .write_all(format!("[Trash Info]\nPath={encoded}\nDeletionDate={date}\n").as_bytes())?;
        return fs::rename(path, files.join(&unique)).inspect_err(|_| {
            let _ = fs::remove_file(&info_path);
        });
    }
    unreachable!("there's always another number")
}

/// The newest entries in `dir`, newest first, without hidden files or unfinished downloads.
pub fn recent_downloads(dir: &Path, limit: usize) -> Vec<Download> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut downloads: Vec<(SystemTime, Download)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let unfinished = [".part", ".crdownload", ".download"]
                .iter()
                .any(|suffix| name.ends_with(suffix));
            if name.starts_with('.') || unfinished {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            // When it was added, if the filesystem knows; otherwise when it last changed.
            let time = metadata.created().or_else(|_| metadata.modified()).ok()?;
            Some((
                time,
                Download {
                    path: entry.path(),
                    name,
                    is_dir: metadata.is_dir(),
                },
            ))
        })
        .collect();

    downloads.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    downloads
        .into_iter()
        .take(limit)
        .map(|(_, download)| download)
        .collect()
}

/// A freedesktop icon name for the download, by its extension.
pub fn icon_name(download: &Download) -> &'static str {
    if download.is_dir {
        return "folder";
    }
    let extension = download
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "heic" | "tiff" => {
            "image-x-generic"
        }
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "video-x-generic",
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" => "audio-x-generic",
        "pdf" => "application-pdf",
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" | "zst" => "package-x-generic",
        "html" | "htm" => "text-html",
        "iso" | "img" => "media-optical",
        "appimage" | "deb" | "rpm" | "exe" | "msi" => "application-x-executable",
        "txt" | "md" | "log" | "json" | "toml" | "csv" => "text-x-generic",
        _ => "unknown",
    }
}

/// The trash, per the freedesktop trash spec.
pub fn trash_dir() -> PathBuf {
    data_home().join("Trash")
}

/// How many items are in the trash.
pub fn trash_count(trash: &Path) -> usize {
    fs::read_dir(trash.join("files")).map_or(0, |entries| entries.count())
}

/// Permanently deletes everything in the trash, with its trash metadata.
pub fn empty_trash(trash: &Path) -> io::Result<()> {
    for subdir in ["files", "info", "expunged"] {
        let entries = match fs::read_dir(trash.join(subdir)) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        for entry in entries {
            let path = entry?.path();
            // Don't follow symlinks: a link in the trash goes, not what it points to.
            if fs::symlink_metadata(&path)?.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }
    }
    // A cache of trashed directories' sizes, stale now.
    match fs::remove_file(trash.join("directorysizes")) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// A `file://` URI for `path`, with the characters URIs can't hold percent-encoded.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(*byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

pub fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn config_home() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"))
}

fn data_home() -> PathBuf {
    xdg_dir("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local/share"))
}

fn xdg_dir(var: &str) -> Option<PathBuf> {
    env::var_os(var)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}
