use std::{
    fmt::Write as _,
    fs::{self},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{format::DelayedFormat, Local};
use once_cell::sync::Lazy;
use tokio::{
    sync::{
        mpsc::UnboundedReceiver,
        mpsc::{self, UnboundedSender}
    },
    task
};

use crate::logging::rotate::Rotate;

pub mod rotate;

static LOGGER: Lazy<Logger> = Lazy::new(|| Logger::new("default"));

pub struct Logger {
    info_writer: UnboundedSender<String>,
    warn_writer: UnboundedSender<String>,
    error_writer: UnboundedSender<String>,
    debug_writer: UnboundedSender<String>,
}

impl Logger {
    pub fn new(log_name: &str) -> Self {
        Logger {
            info_writer: Self::create_writer(&format!("{}_info", log_name)),
            warn_writer: Self::create_writer(&format!("{}_warn", log_name)),
            error_writer: Self::create_writer(&format!("{}_error", log_name)),
            debug_writer: Self::create_writer(&format!("{}_debug", log_name)),
        }
    }

    pub fn info(&self, log: String) {
        self.send(log, &self.info_writer);
    }

    pub fn warn(&self, log: String) {
        self.send(log, &self.warn_writer);
    }

    pub fn error(&self, log: String) {
        self.send(log, &self.error_writer);
    }

    pub fn debug(&self, log: String) {
        self.send(log, &self.debug_writer);
    }

    pub fn send(&self, msg: String, writer: &UnboundedSender<String>) {
        if let Err(why) = writer.send(msg) {
            error_console(why.to_string());
        }
    }

    fn create_writer(log_name: &str) -> UnboundedSender<String> {
        let log_path = Self::get_log_path(log_name).unwrap_or_else(|| {
            panic!("Failed to create log directory.");
        });

        let (tx, rx) = mpsc::unbounded_channel::<String>();

        task::spawn(Self::process_messages(rx, log_path.display().to_string()));

        tx
    }

    async fn process_messages(mut rx: UnboundedReceiver<String>, log_path: String) {
        let mut msg = String::with_capacity(2048);
        let mut rotate = Rotate::new(log_path);

        while let Some(message) = rx.recv().await {
            let now = Local::now();

            if let Err(why) = writeln!(&mut msg, "{} {}", now.format("%F %X%.6f"), message) {
                error_console(format!("Failed to writeln a message. because:{:#?}", why));
                continue;
            }

            if !rx.is_empty() && msg.len() < 2048 {
                continue;
            }

            msg.push('\n');

            if let Some(writer) = rotate.get_writer(now) {
                if let Ok(mut w) = writer.write() {
                    let to_write = msg.as_bytes();
                    if let Err(why) = w.write_all(to_write) {
                        error_console(format!(
                            "Failed to write msg:{}\r\nbecause:{:#?}",
                            msg, why
                        ));
                    }

                    if let Err(why) = w.flush() {
                        error_console(format!("Failed to flush log file. because:{:#?}", why));
                    }

                    msg.clear();
                }
            }
        }
    }

    fn get_log_path(name: &str) -> Option<PathBuf> {
        let path = Path::new("log");

        if !path.exists() {
            fs::create_dir_all(path).ok()?;
        }

        let mut log_path = PathBuf::from(path);
        log_path.push(format!("%Y-%m-%d_{}.log", name));

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
