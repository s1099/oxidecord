//! Runtime errors and panics, appended to `oxidecord.log` beside the prefs.
//!
//! Release builds have no console, so without this a failure leaves no trace.
//! The `log` facade also catches gpui's own error reports.

use std::{
    backtrace::Backtrace,
    fs::{self, File, OpenOptions},
    io::Write,
    sync::Mutex,
};

use log::{Level, LevelFilter, Log, Metadata, Record};

use super::prefs;

const FILE_NAME: &str = "oxidecord.log";
const OLD_FILE_NAME: &str = "oxidecord.old.log";
/// Past this the log is moved aside at startup, so it can't grow unbounded.
const MAX_BYTES: u64 = 1024 * 1024;

static FILE: Mutex<Option<File>> = Mutex::new(None);
static LOGGER: Logger = Logger;

pub fn init() {
    *FILE.lock().unwrap_or_else(|p| p.into_inner()) = open();
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LevelFilter::Warn);
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        // Captured regardless of RUST_BACKTRACE: a crash report without one is rarely useful.
        let backtrace = Backtrace::force_capture();
        write_line(&format!("PANIC [thread {name}] {info}\n{backtrace}"));
        default_hook(info);
    }));
}

fn open() -> Option<File> {
    let path = prefs::config_file(FILE_NAME)?;
    fs::create_dir_all(path.parent()?).ok()?;
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES)
        && let Some(old) = prefs::config_file(OLD_FILE_NAME)
    {
        _ = fs::rename(&path, old);
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn write_line(line: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    // A poisoned lock is still a usable file, and a panic hook must not panic again.
    let mut file = FILE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(file) = file.as_mut() {
        _ = writeln!(file, "{stamp} {line}");
    }
}

struct Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Warn
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let line = format!("{} [{}] {}", record.level(), record.target(), record.args());
            #[cfg(debug_assertions)]
            eprintln!("{line}");
            write_line(&line);
        }
    }

    fn flush(&self) {
        if let Some(file) = FILE.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            _ = file.flush();
        }
    }
}
