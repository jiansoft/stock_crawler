use chrono::{format::DelayedFormat, DateTime, Local};
use concat_string::concat_string;
use crossbeam_channel::{unbounded, Sender};
use once_cell::sync::Lazy;
use slog::*;
use slog_atomic::*;
use std::path::{Path, PathBuf};
use std::{fs, fs::OpenOptions, thread};

static LOGGER: Lazy<Logger> = Lazy::new(|| Logger::new("default"));

pub struct Logger {
    writer: Sender<LogMessage>,
}

impl Logger {
    fn new(log_name: &str) -> Self {
        let (tx, rx) = unbounded::<LogMessage>();
        let log_path = Self::get_log_path(log_name).unwrap_or_else(|| {
            panic!("Failed to create log directory.");
        });

        //寫入檔案的操作使用另一個線程處理
        thread::spawn(move || {
            let slog = create_slog(log_path.as_path());
            let mut together = String::with_capacity(4096);
            together.push_str("\r\n");

            while let Ok(received) = rx.recv() {
                together.push_str(
                    concat_string!(
                        received.created_at.format("%F %X%.6f").to_string(),
                        " ",
                        received.level.to_string(),
                        " ",
                        received.msg,
                        "\r\n"
                    )
                    .as_str(),
                );

                if rx.is_empty() || together.len() >= 4096 {
                    slog::info!(slog, "{}", together);
                    together.clear();
                    together.push_str("\r\n");
                }
            }
        });

        Logger { writer: tx }
    }

    fn info(&self, log: String) {
        self.send(log::Level::Info, log);
    }

    fn error(&self, log: String) {
        self.send(log::Level::Error, log);
    }

    fn send(&self, level: log::Level, msg: String) {
        if let Err(why) = self.writer.send(LogMessage::new(level, msg)) {
            error_console(why.to_string());
        }
    }

    fn get_log_path(name: &str) -> Option<PathBuf> {
        let path = Path::new("log");

        if !path.exists() {
            fs::create_dir_all(path).ok()?;
        }

        let mut log_path = PathBuf::from(path);
        log_path.push(format!("{}_{}.log", name, Local::now().format("%Y-%m-%d")));

        Some(log_path)
    }
}

pub struct LogMessage {
    pub level: log::Level,
    pub msg: String,
    pub created_at: DateTime<Local>,
}

impl LogMessage {
    pub fn new(level: log::Level, msg: String) -> Self {
        LogMessage {
            level,
            msg,
            created_at: Local::now(),
        }
    }
}

fn create_slog(log_path: &Path) -> slog::Logger {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .truncate(false)
        .open(log_path)
        .unwrap_or_else(|e| {
            panic!("Failed to open log file: {}", e);
        });

    let decorator = slog_term::PlainDecorator::new(file);
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).chan_size(512).build().fuse();
    let drain = AtomicSwitch::new(drain);

    slog::Logger::root(drain, o!())
}

pub fn info_file_async(log: String) {
    LOGGER.info(log);
}

pub fn error_file_async(log: String) {
    LOGGER.error(log);
}

pub fn info_console(log: String) {
    println!(
        "{} Info {}",
        Local::now().format("%Y-%m-%d %H:%M:%S.%3f"),
        log
    );
}

pub fn error_console(log: String) {
    println!(
        "{} Error {}",
        DelayedFormat::to_string(&Local::now().format("%Y-%m-%d %H:%M:%S.%3f")),
        log
    );
}
