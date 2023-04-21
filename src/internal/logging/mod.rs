use chrono::{format::DelayedFormat, Local};
use crossbeam_channel::{unbounded, Sender};
use once_cell::sync::Lazy;
use std::{
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    thread,
};

static LOGGER: Lazy<Logger> = Lazy::new(|| Logger::new("default"));

pub struct Logger {
    info_writer: Sender<String>,
    warn_writer: Sender<String>,
    error_writer: Sender<String>,
    debug_writer: Sender<String>,
}

impl Logger {
    fn new(log_name: &str) -> Self {
        let info_writer = Self::create_writer(&format!("{}_info", log_name));
        let warn_writer = Self::create_writer(&format!("{}_warn", log_name));
        let error_writer = Self::create_writer(&format!("{}_error", log_name));
        let debug_writer = Self::create_writer(&format!("{}_debug", log_name));
        Logger {
            info_writer,
            warn_writer,
            error_writer,
            debug_writer,
        }
    }

    fn info(&self, log: String) {
        self.send(log, &self.info_writer);
    }

    fn warn(&self, log: String) {
        self.send(log, &self.warn_writer);
    }

    fn error(&self, log: String) {
        self.send(log, &self.error_writer);
    }

    fn debug(&self, log: String) {
        self.send(log, &self.debug_writer);
    }

    fn send(&self, msg: String, writer: &Sender<String>) {
        if let Err(why) = writer.send(msg) {
            error_console(why.to_string());
        }
    }

    fn create_writer(log_name: &str) -> Sender<String> {
        let log_path = Self::get_log_path(log_name).unwrap_or_else(|| {
            panic!("Failed to create log directory.");
        });
        let (tx, rx) = unbounded::<String>();

        // 寫入檔案的操作使用另一個線程處理
        thread::spawn(move || {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .truncate(false)
                .open(log_path)
                .unwrap_or_else(|e| {
                    panic!("Failed to open log file: {}", e);
                });

            let mut writer = BufWriter::new(file);
            let mut line = String::with_capacity(2048);

            for received in &rx {
                if writeln!(
                    &mut line,
                    "{} {}",
                    Local::now().format("%F %X%.6f"),
                    received
                )
                .is_err()
                {
                    continue;
                }

                if rx.is_empty() || line.len() >= 2048 {
                    if let Err(why) = writeln!(&mut line) {
                        error_console(format!("Failed to writeln a line. because:{:#?}", why));
                    }

                    if let Err(why) = writer.write_all(line.as_bytes()) {
                        error_console(format!(
                            "Failed to write to log file. because:{:#?}\r\nmsg:{}",
                            why, line
                        ));
                    }

                    if let Err(why) = writer.flush() {
                        error_console(format!("Failed to flush log file. because:{:#?}", why));
                    }

                    line.clear();
                }
            }
        });

        tx
    }

    fn get_log_path(name: &str) -> Option<PathBuf> {
        let path = Path::new("log");

        if !path.exists() {
            fs::create_dir_all(path).ok()?;
        }

        let mut log_path = PathBuf::from(path);
        log_path.push(format!("{}_{}.log", Local::now().format("%Y-%m-%d"), name));

        Some(log_path)
    }
}

pub fn info_file_async(log: String) {
    LOGGER.info(log);
}

pub fn warn_file_async(log: String) {
    LOGGER.warn(log);
}

pub fn error_file_async(log: String) {
    LOGGER.error(log);
}

pub fn debug_file_async(log: String) {
    LOGGER.debug(log);
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
