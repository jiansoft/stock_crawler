use chrono::{format::DelayedFormat, DateTime, Local};
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
    writer: Sender<LogMessage>,
}

impl Logger {
    fn new(log_name: &str) -> Self {
        let log_path = Self::get_log_path(log_name).unwrap_or_else(|| {
            panic!("Failed to create log directory.");
        });
        let (tx, rx) = unbounded::<LogMessage>();

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
            let mut line = String::with_capacity(4096);

            while let Ok(received) = rx.recv() {
                if writeln!(
                    &mut line,
                    "{} {} {}",
                    received.created_at.format("%F %X%.6f"),
                    received.level,
                    received.msg
                )
                .is_err()
                {
                    continue;
                }

                if rx.is_empty() || line.len() >= 4096 {
                    /* match writeln!(&mut line) {
                        Ok(_) => {
                            writer
                                .write_all(line.as_bytes())
                                .expect("Failed to write to log file.");
                            writer.flush().expect("Failed to flush log file.");
                        }
                        Err(why) => {
                            info_console(format!("Failed to format log line. because:{:#?}", why));
                            info_console(line.clone())
                        }
                    }*/
                    if writer.write_all(line.as_bytes()).is_err() {
                        //.expect("Failed to write to log file.");
                        info_console(line.clone())
                    }

                    if writer.flush().is_err() {
                        //.expect("Failed to flush log file.");
                        info_console(line.clone())
                    }
                    //writeln!(&mut line).expect("Failed to format log line.");

                    line.clear();
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
