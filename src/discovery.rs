use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

use crate::config;

#[derive(Debug, Clone)]
pub struct ImageFile {
    pub path: PathBuf,
    pub mtime: i64,
}

pub fn find_images() -> Vec<ImageFile> {
    find_images_in(&config::wallpaper_dir())
}

pub fn find_images_in(dir: &str) -> Vec<ImageFile> {
    WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && is_jpeg(e.path()))
        .filter_map(|e| {
            let mtime = get_mtime(e.path()).ok()?;
            Some(ImageFile {
                path: e.path().to_path_buf(),
                mtime,
            })
        })
        .collect()
}

pub fn find_by_basename(basename: &str) -> Option<PathBuf> {
    find_by_basename_in(basename, &config::wallpaper_dir())
}

pub fn find_by_basename_in(basename: &str, dir: &str) -> Option<PathBuf> {
    WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_type().is_file() && e.file_name().to_str() == Some(basename))
        .map(|e| e.path().to_path_buf())
}

fn is_jpeg(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("jpg") || s.eq_ignore_ascii_case("jpeg"))
        .unwrap_or(false)
}

pub fn get_mtime(path: &Path) -> std::io::Result<i64> {
    let metadata = fs::metadata(path)?;
    let mtime = metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(mtime)
}
