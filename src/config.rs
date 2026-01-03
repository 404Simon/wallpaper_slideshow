use std::env;

pub const DEFAULT_WALLPAPER_DIR: &str =
    "/home/simon/dotfiles/wallpaper_slideshow/wallpapers/norway";
pub const DEFAULT_HISTORY_LOG: &str = "/home/simon/.cache/wallpaper_history.log";
pub const DEFAULT_CACHE_DB: &str = "/home/simon/.cache/wallpaper_exif_cache.db";
pub const HISTORY_SIZE: usize = 25;

pub fn wallpaper_dir() -> String {
    env::var("WALLPAPER_DIR").unwrap_or_else(|_| DEFAULT_WALLPAPER_DIR.to_string())
}

pub fn history_log() -> String {
    env::var("WALLPAPER_HISTORY_LOG").unwrap_or_else(|_| DEFAULT_HISTORY_LOG.to_string())
}

pub fn cache_db() -> String {
    env::var("WALLPAPER_CACHE_DB").unwrap_or_else(|_| DEFAULT_CACHE_DB.to_string())
}
