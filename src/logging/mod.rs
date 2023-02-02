use chrono::{format::DelayedFormat, DateTime, Local};
use concat_string::concat_string;
use once_cell::sync::Lazy;
use slog::*;
use slog_atomic::*;
use std::{fs, fs::OpenOptions, thread};

static LOGGER: Lazy<Logger> = Lazy::new(Default::default);

pub struct Logger {
    writer: flume::Sender<LogMessage>,
}

impl Logger {
    fn new() -> Self {
        let (tx, rx) = flume::unbounded::<LogMessage>();
        //寫入檔案的操作使用另一個線程處理
        thread::spawn(move || {
            let slog = create_slog("async");
            /*let mut messages: Vec<LogMessage> = Vec::with_capacity(20);
            for received in rx.iter() {
                messages.push(received);

                //info!(slog, "rx.len()={} messages.len()={}",rx.len(),messages.len());
                if rx.len() != 0 && messages.len() < 20 {
                    continue;
                }

                let mut together = String::with_capacity(2048);
                together.push_str("\n");
                for message in messages.iter() {
                    let msg = concat_string!(
                        message.created_at.format("%F %X%.6f").to_string(),
                        " ",
                        message.level.to_string(),
                        " ",
                        message.msg,
                        "\r\n"
                    );
                    together.push_str(msg.as_str());
                }

                info!(slog, "{}", together);
                messages.clear();
            }
*/
            let mut together = String::with_capacity(2048);
            together.push_str("\r\n");

            for received in rx.iter() {
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

                if rx.len() != 0 && together.len() < 2048 {
                    continue;
                }

                info!(slog, "{}", together);
                together.clear();
                together.push_str("\r\n");
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
}

impl Default for Logger {
    fn default() -> Self {
        Logger::new()
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

fn create_slog(name: &str) -> slog::Logger {
    fs::create_dir_all("log").unwrap();

    let today = Local::now().format("%Y-%m-%d");
    //-{:0>2}-{:0>2}
    let log_path = format!("log/{}_{}.log", name, today);

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .truncate(false)
        .open(log_path)
        .unwrap();

    let decorator = slog_term::PlainDecorator::new(file);
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).chan_size(512).build().fuse();
    // `AtomicSwitch` is a drain that wraps other drain and allows to change
    // it atomically in runtime.
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
        Local::now().format("%Y-%m-%d %H:%M:%S.%3f").to_string(),
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
