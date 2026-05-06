//! 非同步檔案與主控台日誌工具。

use std::{
    fmt::Write as _,
    fs::{self},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    thread,
};

use chrono::{format::DelayedFormat, Local};
use once_cell::sync::Lazy;
use tokio::{
    runtime::Builder,
    sync::mpsc::{self, error::TrySendError, Receiver, Sender},
};

use crate::logging::rotate::Rotate;
use crate::util::atomic::decrement_atomic_usize;

/// 日誌檔輪轉模組。
pub mod rotate;

/// 全域預設 logger。
static LOGGER: Lazy<Logger> = Lazy::new(|| Logger::new("default"));
/// 每個 logger level 的佇列上限，避免高流量時無界吃記憶體。
const LOG_CHANNEL_CAPACITY: usize = 2048;

/// logger 執行期摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoggerRuntimeStatus {
    /// 目前仍在 queue 內、尚未被寫入檔案的訊息數量。
    pub queued_messages: usize,
    /// 自程序啟動以來，已成功寫入檔案的訊息總數。
    pub processed_messages: u64,
    /// 因 queue 已滿或 writer 已關閉而被丟棄的訊息總數。
    pub dropped_messages: u64,
    /// 這份摘要所涵蓋 queue 的總容量。
    pub channel_capacity: usize,
}

#[derive(Debug)]
struct LoggerWriterStats {
    queued_messages: AtomicUsize,
    processed_messages: AtomicU64,
    dropped_messages: AtomicU64,
    channel_capacity: usize,
}

impl LoggerWriterStats {
    fn new(channel_capacity: usize) -> Self {
        Self {
            queued_messages: AtomicUsize::new(0),
            processed_messages: AtomicU64::new(0),
            dropped_messages: AtomicU64::new(0),
            channel_capacity,
        }
    }

    fn snapshot(&self) -> LoggerRuntimeStatus {
        LoggerRuntimeStatus {
            queued_messages: self.queued_messages.load(Ordering::Relaxed),
            processed_messages: self.processed_messages.load(Ordering::Relaxed),
            dropped_messages: self.dropped_messages.load(Ordering::Relaxed),
            channel_capacity: self.channel_capacity,
        }
    }
}

#[derive(Clone)]
struct AsyncLogWriter {
    sender: Sender<String>,
    stats: Arc<LoggerWriterStats>,
}

impl AsyncLogWriter {
    fn diagnostics_snapshot(&self) -> LoggerRuntimeStatus {
        self.stats.snapshot()
    }
}

/// 依等級分流的非同步 logger。
pub struct Logger {
    /// `info` 級別輸出通道。
    info_writer: AsyncLogWriter,
    /// `warn` 級別輸出通道。
    warn_writer: AsyncLogWriter,
    /// `error` 級別輸出通道。
    error_writer: AsyncLogWriter,
    /// `debug` 級別輸出通道。
    debug_writer: AsyncLogWriter,
}

impl Logger {
    /// 建立一組以 `log_name` 為前綴的 logger。
    pub fn new(log_name: &str) -> Self {
        Logger {
            info_writer: Self::create_writer(&format!("{}_info", log_name)),
            warn_writer: Self::create_writer(&format!("{}_warn", log_name)),
            error_writer: Self::create_writer(&format!("{}_error", log_name)),
            debug_writer: Self::create_writer(&format!("{}_debug", log_name)),
        }
    }

    /// 非同步寫入 `info` 等級訊息。
    pub fn info<S: Into<String>>(&self, log: S) {
        self.send(log.into(), &self.info_writer);
    }

    /// 非同步寫入 `warn` 等級訊息。
    pub fn warn<S: Into<String>>(&self, log: S) {
        self.send(log.into(), &self.warn_writer);
    }

    /// 非同步寫入 `error` 等級訊息。
    pub fn error<S: Into<String>>(&self, log: S) {
        self.send(log.into(), &self.error_writer);
    }

    /// 非同步寫入 `debug` 等級訊息。
    pub fn debug<S: Into<String>>(&self, log: S) {
        self.send(log.into(), &self.debug_writer);
    }

    /// 將訊息送入指定 writer 佇列。
    fn send(&self, msg: String, writer: &AsyncLogWriter) {
        writer.stats.queued_messages.fetch_add(1, Ordering::Relaxed);

        match writer.sender.try_send(msg) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Closed(_)) => {
                decrement_atomic_usize(&writer.stats.queued_messages);
                writer
                    .stats
                    .dropped_messages
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// 取得此 logger 目前的 queue / drop 摘要。
    pub fn diagnostics_snapshot(&self) -> LoggerRuntimeStatus {
        let info = self.info_writer.diagnostics_snapshot();
        let warn = self.warn_writer.diagnostics_snapshot();
        let error = self.error_writer.diagnostics_snapshot();
        let debug = self.debug_writer.diagnostics_snapshot();

        LoggerRuntimeStatus {
            queued_messages: info.queued_messages
                + warn.queued_messages
                + error.queued_messages
                + debug.queued_messages,
            processed_messages: info.processed_messages
                + warn.processed_messages
                + error.processed_messages
                + debug.processed_messages,
            dropped_messages: info.dropped_messages
                + warn.dropped_messages
                + error.dropped_messages
                + debug.dropped_messages,
            channel_capacity: info.channel_capacity
                + warn.channel_capacity
                + error.channel_capacity
                + debug.channel_capacity,
        }
    }

    fn create_writer(log_name: &str) -> AsyncLogWriter {
        let log_path = Self::get_log_path(log_name).unwrap_or_else(|| {
            panic!("Failed to create log directory.");
        });

        let (tx, rx) = mpsc::channel::<String>(LOG_CHANNEL_CAPACITY);
        let stats = Arc::new(LoggerWriterStats::new(LOG_CHANNEL_CAPACITY));

        // Use a dedicated thread + runtime so logger workers outlive per-test tokio runtimes.
        let path = log_path.display().to_string();
        let worker_stats = Arc::clone(&stats);
        thread::spawn(move || {
            let rt = Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|e| panic!("Failed to build logger runtime: {e}"));
            rt.block_on(Self::process_messages(rx, path, worker_stats));
        });

        AsyncLogWriter { sender: tx, stats }
    }

    async fn process_messages(
        mut rx: Receiver<String>,
        log_path: String,
        stats: Arc<LoggerWriterStats>,
    ) {
        let mut msg = String::with_capacity(2048);
        let mut rotate = Rotate::new(log_path);

        while let Some(message) = rx.recv().await {
            decrement_atomic_usize(&stats.queued_messages);
            stats.processed_messages.fetch_add(1, Ordering::Relaxed);
            let now = Local::now();

            if let Err(why) = writeln!(&mut msg, "{} {}", now.format("%F %X%.6f"), message) {
                error_console(format!("Failed to writeln a message. because:{:#?}", why));
                continue;
            }

            if !rx.is_empty() && msg.len() < 2048 {
                continue;
            }

            msg.push('\n');

            flush_log_buffer(&mut rotate, now, &mut msg);
        }

        if !msg.is_empty() {
            let now = Local::now();
            msg.push('\n');
            flush_log_buffer(&mut rotate, now, &mut msg);
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

fn flush_log_buffer(rotate: &mut Rotate, now: chrono::DateTime<Local>, msg: &mut String) {
    if let Some(writer) = rotate.get_writer(now) {
        if let Ok(mut w) = writer.write() {
            let to_write = msg.as_bytes();
            if let Err(why) = w.write_all(to_write) {
                error_console(format!("Failed to write msg:{}\r\nbecause:{:#?}", msg, why));
            }

            if let Err(why) = w.flush() {
                error_console(format!("Failed to flush log file. because:{:#?}", why));
            }

            msg.clear();
        }
    }
}

/// 使用全域 logger 寫入 `info` 等級檔案日誌。
pub fn info_file_async<S: Into<String>>(log: S) {
    LOGGER.info(log.into());
}

/// 使用全域 logger 寫入 `warn` 等級檔案日誌。
pub fn warn_file_async<S: Into<String>>(log: S) {
    LOGGER.warn(log.into());
}

/// 使用全域 logger 寫入 `error` 等級檔案日誌。
pub fn error_file_async<S: Into<String>>(log: S) {
    LOGGER.error(log.into());
}

/// 使用全域 logger 寫入 `debug` 等級檔案日誌。
pub fn debug_file_async<S: Into<String>>(log: S) {
    LOGGER.debug(log.into());
}

/// 取得預設 logger 的執行期摘要。
pub fn diagnostics_snapshot() -> LoggerRuntimeStatus {
    LOGGER.diagnostics_snapshot()
}

/// 直接輸出 `info` 等級到標準輸出。
pub fn info_console(log: String) {
    println!(
        "{} Info {}",
        Local::now().format("%Y-%m-%d %H:%M:%S.%3f"),
        log
    );
}

/// 直接輸出 `error` 等級到標準輸出。
pub fn error_console(log: String) {
    println!(
        "{} Error {}",
        DelayedFormat::to_string(&Local::now().format("%Y-%m-%d %H:%M:%S.%3f")),
        log
    );
}
