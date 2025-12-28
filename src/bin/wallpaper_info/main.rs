mod color;
mod exif;

use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use base64::Engine;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use image::ImageReader;

use color::{extract_palette, ColorPalette, COLOR_RESET};
use exif::ExifInfo;

const DEFAULT_WALLPAPER_DIR: &str = "/home/simon/dotfiles/wallpapers/norway";
const DEFAULT_HISTORY_LOG: &str = "/home/simon/.cache/wallpaper_history.log";

fn get_wallpaper_dir() -> String {
    env::var("WALLPAPER_DIR").unwrap_or_else(|_| DEFAULT_WALLPAPER_DIR.to_string())
}

fn get_history_log() -> String {
    env::var("WALLPAPER_HISTORY_LOG").unwrap_or_else(|_| DEFAULT_HISTORY_LOG.to_string())
}

static IS_TMUX: LazyLock<bool> = LazyLock::new(|| {
    env::var("TMUX").is_ok_and(|v| !v.is_empty())
        && env::var("TMUX_PANE").is_ok_and(|v| !v.is_empty())
});

struct WallpaperHistory {
    entries: Vec<String>,
    current_index: usize,
}

impl WallpaperHistory {
    fn load() -> Option<Self> {
        let history_log = get_history_log();
        let file = File::open(&history_log).ok()?;
        let entries: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
        if entries.is_empty() {
            return None;
        }
        Some(Self {
            current_index: entries.len() - 1,
            entries,
        })
    }

    fn current_basename(&self) -> &str {
        &self.entries[self.current_index]
    }
    fn go_previous(&mut self) -> bool {
        if self.current_index > 0 {
            self.current_index -= 1;
            true
        } else {
            false
        }
    }
    fn go_next(&mut self) -> bool {
        if self.current_index < self.entries.len() - 1 {
            self.current_index += 1;
            true
        } else {
            false
        }
    }
    fn position_str(&self) -> String {
        format!("{}/{}", self.current_index + 1, self.entries.len())
    }

    fn current_path(&self) -> Option<PathBuf> {
        let basename = self.current_basename();
        walkdir::WalkDir::new(get_wallpaper_dir())
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .find(|e| e.file_type().is_file() && e.file_name().to_str() == Some(basename))
            .map(|e| e.path().to_path_buf())
    }
}

struct ImageMeta {
    width: u32,
    height: u32,
    file_size: u64,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("wallpaper-info {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        r#"wallpaper-info {}
Display current wallpaper with EXIF metadata

USAGE:
    wallpaper-info [OPTIONS]

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version information

ENVIRONMENT VARIABLES:
    WALLPAPER_DIR          Directory containing wallpaper images
                           Default: {}
    WALLPAPER_HISTORY_LOG  Path to wallpaper history log file
                           Default: {}

KEYBINDINGS:
    q, Esc    Quit the application
    m         Open location in Google Maps (if GPS data available)
    c         Copy GPS coordinates to clipboard (if available)
    Left/Up   Show previous wallpaper from history
    Right/Down Show next wallpaper from history
"#,
        env!("CARGO_PKG_VERSION"),
        DEFAULT_WALLPAPER_DIR,
        DEFAULT_HISTORY_LOG
    );
}

fn run() -> io::Result<()> {
    let mut history = WallpaperHistory::load()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No wallpaper history found"))?;

    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;

    let mut current_exif = display_wallpaper(&mut stdout, &history)?;

    loop {
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key {
                    KeyEvent {
                        code: KeyCode::Char('q'),
                        ..
                    }
                    | KeyEvent {
                        code: KeyCode::Esc, ..
                    }
                    | KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } => break,

                    KeyEvent {
                        code: KeyCode::Char('m'),
                        ..
                    } => {
                        if let Some(url) = current_exif.maps_url() {
                            let _ = open_url(&url);
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        ..
                    } => {
                        if let (Some(lat), Some(lon)) =
                            (current_exif.gps_latitude, current_exif.gps_longitude)
                        {
                            let _ = copy_to_clipboard(&format!("{:.6}, {:.6}", lat, lon));
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k'),
                        ..
                    } => {
                        if history.go_previous() {
                            current_exif = display_wallpaper(&mut stdout, &history)?;
                        }
                    }
                    KeyEvent {
                        code:
                            KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j'),
                        ..
                    } => {
                        if history.go_next() {
                            current_exif = display_wallpaper(&mut stdout, &history)?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    write_kitty_escape(&mut stdout, "\x1b_Ga=d,d=A,q=2\x1b\\")?;
    terminal::disable_raw_mode()?;
    stdout.execute(LeaveAlternateScreen)?;
    Ok(())
}

fn display_wallpaper(stdout: &mut io::Stdout, history: &WallpaperHistory) -> io::Result<ExifInfo> {
    let path = history.current_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("Could not find: {}", history.current_basename()),
        )
    })?;

    let exif_info = exif::extract(&path);
    let file_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let (term_width, term_height) = terminal::size().unwrap_or((80, 24));

    let image = ImageReader::new(Cursor::new(fs::read(&path)?))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.decode().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Failed to decode image"))?;

    let palette = extract_palette(&image);
    let meta = ImageMeta {
        width: image.width(),
        height: image.height(),
        file_size,
    };

    let window_size = terminal::window_size().unwrap_or(terminal::WindowSize {
        width: 1920,
        height: 1080,
        rows: term_height,
        columns: term_width,
    });

    let cell_width = window_size.width as f64 / window_size.columns as f64;
    let cell_height = window_size.height as f64 / window_size.rows as f64;
    let panel_height: u16 = 12;
    let image_area_height = term_height.saturating_sub(panel_height + 1);

    let scale = (term_width as f64 * cell_width / image.width() as f64)
        .min(image_area_height as f64 * cell_height / image.height() as f64);

    let (target_w, target_h) = (
        (image.width() as f64 * scale) as u32,
        (image.height() as f64 * scale) as u32,
    );
    let resized = image.resize(target_w, target_h, image::imageops::FilterType::Lanczos3);

    let cells_w = (target_w as f64 / cell_width).ceil() as u16;
    let cells_h = (target_h as f64 / cell_height).ceil() as u16;
    let h_offset = (term_width.saturating_sub(cells_w)) / 2;
    let v_offset = (image_area_height.saturating_sub(cells_h)) / 2;

    let bg = &palette.background;
    write!(stdout, "\x1b[48;2;{};{};{}m\x1b[2J\x1b[H", bg.r, bg.g, bg.b)?;
    display_kitty_image(
        stdout,
        &resized,
        cells_w,
        cells_h,
        h_offset + 1,
        v_offset + 1,
    )?;
    display_panel(
        stdout,
        &path,
        &exif_info,
        &meta,
        &palette,
        term_width,
        term_height,
        panel_height,
        &history.position_str(),
    )?;
    stdout.flush()?;

    Ok(exif_info)
}

fn display_kitty_image(
    w: &mut impl Write,
    img: &image::DynamicImage,
    cells_w: u16,
    cells_h: u16,
    col: u16,
    row: u16,
) -> io::Result<()> {
    let rgba = img.to_rgba8();
    let (width, height) = (img.width(), img.height());

    write_kitty_escape(w, "\x1b_Ga=d,d=A,q=2\x1b\\")?;
    w.flush()?;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(rgba.as_raw())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(encoder.finish()?);

    write!(w, "\x1b[{};{}H", row, col)?;

    let mut chars = encoded.chars().peekable();
    let first: String = chars.by_ref().take(4096).collect();
    let more = if chars.peek().is_some() { 1 } else { 0 };

    write_kitty_escape(
        w,
        &format!(
            "\x1b_Ga=T,f=32,t=d,m={},q=2,o=z,s={},v={},c={},r={};{}\x1b\\",
            more, width, height, cells_w, cells_h, first
        ),
    )?;

    while chars.peek().is_some() {
        let chunk: String = chars.by_ref().take(4096).collect();
        let more = if chars.peek().is_some() { 1 } else { 0 };
        write_kitty_escape(w, &format!("\x1b_Gm={};{}\x1b\\", more, chunk))?;
    }

    w.flush()
}

fn write_kitty_escape(w: &mut impl Write, content: &str) -> io::Result<()> {
    if *IS_TMUX {
        write!(w, "\x1bPtmux;")?;
        for c in content.chars() {
            if c == '\x1b' {
                write!(w, "\x1b\x1b")?;
            } else {
                write!(w, "{}", c)?;
            }
        }
        write!(w, "\x1b\\")?;
    } else {
        write!(w, "{}", content)?;
    }
    Ok(())
}

fn display_panel(
    w: &mut impl Write,
    path: &Path,
    info: &ExifInfo,
    meta: &ImageMeta,
    palette: &ColorPalette,
    term_width: u16,
    term_height: u16,
    panel_height: u16,
    position: &str,
) -> io::Result<()> {
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown");
    let panel_start = term_height.saturating_sub(panel_height);
    let (accent, secondary, dim, text) = (
        palette.accent.as_fg(),
        palette.secondary.as_fg(),
        palette.dim.as_fg(),
        palette.text.as_fg(),
    );
    let bg = palette.background.darken(0.3).as_bg();

    // Draw background
    for row in panel_start..=term_height {
        write!(
            w,
            "\x1b[{};1H{}{}",
            row,
            bg,
            " ".repeat(term_width as usize)
        )?;
    }

    // Border
    write!(
        w,
        "\x1b[{};1H{}{}{}",
        panel_start,
        palette.accent.muted().as_fg(),
        "─".repeat(term_width as usize),
        COLOR_RESET
    )?;

    let left = 3u16;
    let mut row = panel_start + 1;

    // Title row
    write!(
        w,
        "\x1b[{};{}H{}{}{}",
        row,
        left,
        bg,
        accent,
        truncate(filename, term_width as usize / 2)
    )?;
    let pos_text = format!("[{}]", position);
    write!(
        w,
        "\x1b[{};{}H{}{}{}",
        row,
        term_width.saturating_sub(pos_text.len() as u16 + 3),
        bg,
        dim,
        pos_text
    )?;
    row += 1;

    // Dimensions
    write!(
        w,
        "\x1b[{};{}H{}{}{}x{}  {}{}{}",
        row,
        left,
        bg,
        dim,
        meta.width,
        meta.height,
        secondary,
        format_size(meta.file_size),
        COLOR_RESET
    )?;
    row += 2;

    let col2 = term_width / 2;

    // Column 1: When & Where
    if let Some(ref dt) = info.datetime {
        write!(
            w,
            "\x1b[{};{}H{}{} When   {}{}{}",
            row, left, bg, accent, text, dt, COLOR_RESET
        )?;
        row += 1;
    }
    if let Some(ref loc) = info.location {
        write!(
            w,
            "\x1b[{};{}H{}{} Where  {}{}{}",
            row, left, bg, accent, text, loc, COLOR_RESET
        )?;
        if info.has_gps() {
            row += 1;
            write!(
                w,
                "\x1b[{};{}H{}{}        Press {}m{} for Maps{}",
                row, left, bg, dim, accent, dim, COLOR_RESET
            )?;
        }
    }

    // Column 2: Camera & Settings
    row = panel_start + 3;
    if let Some(ref cam) = info.camera {
        write!(
            w,
            "\x1b[{};{}H{}{} Camera  {}{}{}",
            row,
            col2,
            bg,
            secondary,
            text,
            truncate(cam, (term_width / 2 - 10) as usize),
            COLOR_RESET
        )?;
        row += 1;
    }
    if let Some(ref lens) = info.lens {
        write!(
            w,
            "\x1b[{};{}H{}{}          {}{}",
            row,
            col2,
            bg,
            dim,
            truncate(lens, (term_width / 2 - 12) as usize),
            COLOR_RESET
        )?;
        row += 1;
    }

    let settings: Vec<&str> = [
        &info.focal_length,
        &info.aperture,
        &info.exposure,
        &info.iso,
    ]
    .iter()
    .filter_map(|o| o.as_ref().map(|s| s.as_str()))
    .collect();
    if !settings.is_empty() {
        write!(w, "\x1b[{};{}H{}{} Settings  ", row, col2, bg, secondary)?;
        for (i, s) in settings.iter().enumerate() {
            if i > 0 {
                write!(w, "{}  ", dim)?;
            }
            write!(w, "{}{}", text, s)?;
        }
        write!(w, "{}", COLOR_RESET)?;
    }

    // Help bar
    write!(
        w,
        "\x1b[{};{}H{} {}</>{}Navigate   {}q{}Quit",
        term_height, left, bg, accent, dim, accent, dim
    )?;
    if info.has_gps() {
        write!(w, "   {}m{}Maps   {}c{}Copy", accent, dim, accent, dim)?;
    }
    write!(w, "{}", COLOR_RESET)?;

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max > 3 {
        format!("{}...", s.chars().take(max - 3).collect::<String>())
    } else {
        s.chars().take(max).collect()
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn open_url(url: &str) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

fn copy_to_clipboard(text: &str) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("wl-copy")
        .arg(text)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Ok(())
}
