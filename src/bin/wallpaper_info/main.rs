mod color;
mod display;

use std::env;
use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;

use wallpaper_slideshow::{WallpaperHistory, DEFAULT_HISTORY_LOG, DEFAULT_WALLPAPER_DIR};

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

    let mut current_exif = display::show_wallpaper(&mut stdout, &history)?;

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
                            current_exif = display::show_wallpaper(&mut stdout, &history)?;
                        }
                    }

                    KeyEvent {
                        code:
                            KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j'),
                        ..
                    } => {
                        if history.go_next() {
                            current_exif = display::show_wallpaper(&mut stdout, &history)?;
                        }
                    }

                    _ => {}
                }
            }
        }
    }

    display::cleanup(&mut stdout)?;
    terminal::disable_raw_mode()?;
    stdout.execute(LeaveAlternateScreen)?;
    Ok(())
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
