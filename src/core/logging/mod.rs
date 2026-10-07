//! 非同步檔案與主控台日誌工具。
//!
//! 檔案分工：本檔是依等級分流的輪轉檔案 logger 與主控台輸出；[`layer`] 把 tracing 事件接到
//! 這個 logger，[`seq`] 負責轉送 Seq。

use std::{
    fmt::Write as _,
    fs::{self},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
};

use chrono::{Local, format::DelayedFormat};
use once_cell::sync::Lazy;
use tokio::{
    runtime::Builder,
    sync::mpsc::{self, Receiver, Sender, error::TrySendError},
};

use crate::core::logging::rotate::Rotate;
use crate::core::util::atomic::decrement_atomic_usize;

/// tracing 事件寫入輪轉檔案與轉送 Seq 的 layer。
mod layer;
/// 日誌檔輪轉模組。
pub mod rotate;
/// Seq 日誌轉送。
mod seq;

pub use layer::{FileLogLayer, file_log_env_filter};
use seq::SeqLogLevel;
pub use seq::init_seq;

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

/// 單一日誌 writer 的 queue 與處理統計。
///
/// 統計值會在主執行緒 enqueue、背景 worker 寫檔與 drop 訊息時更新。
#[derive(Debug)]
struct LoggerWriterStats {
    /// 目前仍在 queue 內、尚未被背景 worker 處理的訊息數。
    queued_messages: AtomicUsize,
    /// 背景 worker 已處理的訊息總數。
    processed_messages: AtomicU64,
    /// 因 queue 滿或通道關閉而丟棄的訊息總數。
    dropped_messages: AtomicU64,
    /// 此 writer 使用的 queue 容量。
    channel_capacity: usize,
}

impl LoggerWriterStats {
    /// 建立指定 queue 容量的統計容器。
    fn new(channel_capacity: usize) -> Self {
        Self {
            queued_messages: AtomicUsize::new(0),
            processed_messages: AtomicU64::new(0),
            dropped_messages: AtomicU64::new(0),
            channel_capacity,
        }
    }

    /// 取得目前統計快照。
    fn snapshot(&self) -> LoggerRuntimeStatus {
        LoggerRuntimeStatus {
            queued_messages: self.queued_messages.load(Ordering::Relaxed),
            processed_messages: self.processed_messages.load(Ordering::Relaxed),
            dropped_messages: self.dropped_messages.load(Ordering::Relaxed),
            channel_capacity: self.channel_capacity,
        }
    }
}

/// 背景日誌 writer 的 enqueue 端與統計資料。
///
/// Clone 時只複製 sender 與 `Arc` 統計資料，不會建立新的背景 worker。
#[derive(Clone)]
struct AsyncLogWriter {
    /// 背景 worker 接收日誌訊息的 channel。
    sender: Sender<String>,
    /// 此 writer 的 queue / drop 統計資料。
    stats: Arc<LoggerWriterStats>,
}

impl AsyncLogWriter {
    /// 回傳此 writer 目前的執行期統計快照。
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
            info_writer: Self::create_writer(&format!("{}_info", log_name), SeqLogLevel::Info),
            warn_writer: Self::create_writer(&format!("{}_warn", log_name), SeqLogLevel::Warn),
            error_writer: Self::create_writer(&format!("{}_error", log_name), SeqLogLevel::Error),
            debug_writer: Self::create_writer(&format!("{}_debug", log_name), SeqLogLevel::Debug),
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

    /// 建立指定檔名與 Seq 等級的背景 writer。
    ///
    /// 每個等級各自擁有獨立 channel 與 thread，避免單一慢速寫入拖累所有等級。
    ///
    /// **禁止在此函式（及 `LOGGER` 初始化路徑上的任何程式碼）存取
    /// `core::config::SETTINGS`。** `SETTINGS` 的初始化過程本身會呼叫
    /// `tracing::error!`（見 `config::App::override_with_env` 對 `TELEGRAM_ALLOWED`
    /// 的解析失敗處理），那會反過來觸發 `LOGGER` 這個 `Lazy` 的初始化，
    /// 形成同執行緒的 Lazy 重入 —— once_cell 的 std 實作在此情況下是**死鎖**
    /// 而非 panic，症狀為啟動時無訊息卡死，且只在該環境變數剛好打錯時才出現。
    ///
    /// 需要設定值時一律由 `main` 讀完設定後推入（見 [`init_file_rotation`]、
    /// [`init_seq`]）。
    fn create_writer(log_name: &str, seq_level: SeqLogLevel) -> AsyncLogWriter {
        let log_path = Self::get_log_path(log_name).unwrap_or_else(|| {
            panic!("Failed to create log directory.");
        });

        let (tx, rx) = mpsc::channel::<String>(LOG_CHANNEL_CAPACITY);
        let stats = Arc::new(LoggerWriterStats::new(LOG_CHANNEL_CAPACITY));

        // 使用專屬 thread 與 runtime，讓 logger worker 不受測試或呼叫端 tokio runtime 生命週期影響。
        // seq_level 保留於此層用於未來擴充（例如 per-level 過濾），目前 Seq 轉送已移至 FileLogLayer。
        let _ = seq_level;
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

    /// 背景處理日誌 queue，負責寫檔。
    ///
    /// 檔案寫入會累積成小批次降低 IO 次數。
    /// Seq 轉送已移至 `FileLogLayer::on_event`，可攜帶完整結構化欄位。
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

            // Seq 轉送已移至 FileLogLayer::on_event，此處只負責寫檔。

            if let Err(why) = writeln!(&mut msg, "{} {}", now.format("%F %X%.6f"), message) {
                error_console(format!("Failed to writeln a message. because:{:#?}", why));
                continue;
            }

            if !rx.is_empty() && msg.len() < 2048 {
                continue;
            }

            // 每行已由 writeln! 換行，批次結尾不能再補 '\n'——舊版每次 flush 多一個空行，
            // 正式機的 warn 日誌一半是空行（2026-10-07：652 行裡 326 行）。
            flush_log_buffer(&mut rotate, now, &mut msg);
        }

        if !msg.is_empty() {
            let now = Local::now();
            flush_log_buffer(&mut rotate, now, &mut msg);
        }
    }

    /// 產生指定 logger 名稱對應的輪轉檔案路徑。
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

/// 套用輪轉日誌檔設定（單檔大小上限與保留天數）。
///
/// 對應 `app.json` 的 `logging.file`，可由 `.env` 的 `LOG_FILE_MAX_SIZE_MB`、
/// `LOG_FILE_MAX_AGE_DAYS` 覆蓋。傳入 `0` 代表未設定，沿用預設（10 MB / 7 天）。
///
/// 應在 `.env` 與設定載入後、開始大量寫日誌前呼叫；設定以全域參數保存，
/// 已建立的背景 writer 也會立即套用。
pub fn init_file_rotation(max_size_mb: u64, max_age_days: i64) {
    rotate::configure(max_size_mb, max_age_days);

    let (size, days) = rotate::effective_settings();
    info_console(format!(
        "Log rotation: max_size={} MB, max_age={} days",
        size / (1024 * 1024),
        days
    ));
}

/// 將累積的日誌文字寫入輪轉檔案。
///
/// 一律走 `Rotate::write_msg`，讓跨日切檔與單檔大小輪轉都能生效
/// （先前直接取 writer 寫入，導致 `current_size` 永遠為 0、大小輪轉從未觸發）。
/// 無論成功與否都清空緩衝，避免寫檔持續失敗時 buffer 無界成長。
fn flush_log_buffer(rotate: &mut Rotate, now: chrono::DateTime<Local>, msg: &mut String) {
    if let Err(why) = rotate.write_msg(now, msg.as_bytes()) {
        error_console(format!("Failed to write msg:{}\r\nbecause:{:#?}", msg, why));
    }

    msg.clear();
}

/// 寫入 `info` 等級日誌（透過 tracing → FileLogLayer → LOGGER）。
pub fn info_file_async<S: Into<String>>(log: S) {
    tracing::info!("{}", log.into());
}

/// 寫入 `warn` 等級日誌（透過 tracing → FileLogLayer → LOGGER）。
pub fn warn_file_async<S: Into<String>>(log: S) {
    tracing::warn!("{}", log.into());
}

/// 寫入 `error` 等級日誌（透過 tracing → FileLogLayer → LOGGER）。
pub fn error_file_async<S: Into<String>>(log: S) {
    tracing::error!("{}", log.into());
}

/// 寫入 `debug` 等級日誌（透過 tracing → FileLogLayer → LOGGER）。
pub fn debug_file_async<S: Into<String>>(log: S) {
    tracing::debug!("{}", log.into());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 日誌檔路徑帶有 `Rotate` 用來替換的日期樣板與 logger 名稱。
    #[test]
    fn log_path_carries_the_date_template_and_logger_name() {
        let path = Logger::get_log_path("default_info").expect("路徑應可建立");

        assert_eq!(path.parent(), Some(Path::new("log")));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("%Y-%m-%d_default_info.log")
        );
    }

    /// 統計快照如實反映各計數器目前的值。
    #[test]
    fn writer_stats_snapshot_reports_current_counters() {
        let stats = LoggerWriterStats::new(LOG_CHANNEL_CAPACITY);
        stats.queued_messages.store(2, Ordering::Relaxed);
        stats.processed_messages.store(7, Ordering::Relaxed);
        stats.dropped_messages.store(1, Ordering::Relaxed);

        assert_eq!(
            stats.snapshot(),
            LoggerRuntimeStatus {
                queued_messages: 2,
                processed_messages: 7,
                dropped_messages: 1,
                channel_capacity: LOG_CHANNEL_CAPACITY,
            }
        );
    }

    /// 佇列已滿時訊息會被丟棄，並且不留下「排隊中」的假象。
    ///
    /// 這裡直接建構 writer（不經 `Logger::new`），避免起背景 thread 真的寫檔。
    #[tokio::test]
    async fn send_drops_the_message_when_the_queue_is_full() {
        let (sender, _rx) = mpsc::channel::<String>(1);
        let writer = AsyncLogWriter {
            sender,
            stats: Arc::new(LoggerWriterStats::new(1)),
        };
        let logger = Logger {
            info_writer: writer.clone(),
            warn_writer: writer.clone(),
            error_writer: writer.clone(),
            debug_writer: writer.clone(),
        };

        // 容量 1：第一筆進佇列，第二筆無處可放只能丟棄。
        logger.send("first".to_string(), &writer);
        logger.send("second".to_string(), &writer);

        let snapshot = writer.diagnostics_snapshot();
        assert_eq!(snapshot.queued_messages, 1);
        assert_eq!(snapshot.dropped_messages, 1);
        assert_eq!(snapshot.channel_capacity, 1);
    }

    /// 接收端關閉後所有訊息都算丟棄，且不可 panic。
    #[tokio::test]
    async fn send_counts_drops_after_the_receiver_is_closed() {
        let (sender, rx) = mpsc::channel::<String>(4);
        drop(rx);
        let writer = AsyncLogWriter {
            sender,
            stats: Arc::new(LoggerWriterStats::new(4)),
        };
        let logger = Logger {
            info_writer: writer.clone(),
            warn_writer: writer.clone(),
            error_writer: writer.clone(),
            debug_writer: writer.clone(),
        };

        logger.error("boom".to_string());

        let snapshot = writer.diagnostics_snapshot();
        assert_eq!(snapshot.queued_messages, 0);
        assert_eq!(snapshot.dropped_messages, 1);
    }

    /// 四個等級的統計會彙總成單一摘要。
    #[tokio::test]
    async fn logger_snapshot_aggregates_every_level() {
        let make = || {
            let (sender, rx) = mpsc::channel::<String>(8);
            (
                AsyncLogWriter {
                    sender,
                    stats: Arc::new(LoggerWriterStats::new(8)),
                },
                rx,
            )
        };
        // receiver 必須留著：一旦 drop，channel 關閉會讓訊息全部改記為丟棄。
        let (info, _info_rx) = make();
        let (warn, _warn_rx) = make();
        let (error, _error_rx) = make();
        let (debug, _debug_rx) = make();
        let logger = Logger {
            info_writer: info,
            warn_writer: warn,
            error_writer: error,
            debug_writer: debug,
        };

        logger.info("i");
        logger.warn("w");
        logger.error("e");
        logger.debug("d");

        let snapshot = logger.diagnostics_snapshot();
        assert_eq!(snapshot.queued_messages, 4);
        assert_eq!(snapshot.dropped_messages, 0);
        assert_eq!(snapshot.channel_capacity, 32);
    }
}
