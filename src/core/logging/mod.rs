//! 非同步檔案與主控台日誌工具。

use std::{
    fmt::Write as _,
    fs::{self},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    thread,
};

use chrono::{format::DelayedFormat, Local, SecondsFormat, Utc};
use once_cell::sync::{Lazy, OnceCell};
use reqwest::Client;
use serde::Serialize;
use tokio::{
    runtime::Builder,
    sync::mpsc::{self, error::TrySendError, Receiver, Sender},
    time::{self, Duration},
};

use crate::core::logging::rotate::Rotate;
use crate::core::util::{atomic::decrement_atomic_usize, ensure_rustls_crypto_provider};

/// 日誌檔輪轉模組。
pub mod rotate;

/// 全域預設 logger。
static LOGGER: Lazy<Logger> = Lazy::new(|| Logger::new("default"));
/// 每個 logger level 的佇列上限，避免高流量時無界吃記憶體。
const LOG_CHANNEL_CAPACITY: usize = 2048;
/// Seq 背景發送佇列上限，保留足夠緩衝避免短時間尖峰阻塞主流程。
const SEQ_CHANNEL_CAPACITY: usize = 10_000;
/// Seq 批次送出的間隔毫秒數。
const SEQ_FLUSH_INTERVAL_MS: u64 = 1_000;
/// Seq 單次批次送出的最大事件數。
const SEQ_BATCH_EVENT_LIMIT: usize = 512;
/// 送到 Seq 的服務名稱。
///
/// 這個名稱不使用 Cargo package name，避免 Seq 事件顯示為 `stock_crawler`。
const SEQ_SERVICE_NAME: &str = "stock_rust";
/// Seq 是否已完成初始化；未啟用時只保留既有檔案日誌行為。
static SEQ_LOGGING_ENABLED: AtomicBool = AtomicBool::new(false);
/// Seq 背景 sender；存在代表已建立專屬背景 worker。
static SEQ_SENDER: OnceCell<Sender<SeqEvent>> = OnceCell::new();

/// 寫入 Seq 時保留的原始日誌等級。
///
/// 這個 enum 只用於把既有檔案日誌等級轉成 Seq 事件與附加屬性。
#[derive(Debug, Clone, Copy)]
enum SeqLogLevel {
    /// 一般資訊訊息。
    Info,
    /// 需要注意但不一定中斷流程的警告。
    Warn,
    /// 流程失敗或外部服務異常。
    Error,
    /// 開發與診斷用的詳細訊息。
    Debug,
}

impl SeqLogLevel {
    /// 回傳本專案原始日誌等級名稱。
    ///
    /// 此名稱會以 `RustLogLevel` 屬性送到 Seq，方便沿用本專案既有等級查詢。
    fn as_rust_level(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Warn => "Warn",
            Self::Error => "Error",
            Self::Debug => "Debug",
        }
    }

    /// 回傳 Seq / Serilog 可辨識的等級名稱。
    ///
    /// Seq 的 CLEF ingestion 使用 `Information`、`Warning`、`Error`、`Debug`
    /// 等名稱，因此這裡和本專案檔案日誌的簡寫分開處理。
    fn as_seq_level(self) -> &'static str {
        match self {
            Self::Info => "Information",
            Self::Warn => "Warning",
            Self::Error => "Error",
            Self::Debug => "Debug",
        }
    }
}

/// 送往 Seq 的 CLEF 事件。
///
/// 欄位刻意不包含多餘的應用程式名稱欄位，避免 Seq 畫面重複顯示服務識別。
#[derive(Debug, Serialize)]
struct SeqEvent {
    /// Seq 標準事件時間欄位，使用 UTC RFC3339。
    #[serde(rename = "@t")]
    timestamp: String,
    /// Seq 標準訊息樣板欄位。
    #[serde(rename = "@mt")]
    message_template: String,
    /// Seq 標準等級欄位。
    #[serde(rename = "@l")]
    level: &'static str,
    /// 服務名稱；作為 Seq 查詢與分組欄位。
    service: &'static str,
    /// 本專案原始日誌等級。
    #[serde(rename = "RustLogLevel")]
    rust_log_level: &'static str,
    /// 事件來源 logger。
    #[serde(rename = "Logger")]
    logger: &'static str,
}

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
    fn create_writer(log_name: &str, seq_level: SeqLogLevel) -> AsyncLogWriter {
        let log_path = Self::get_log_path(log_name).unwrap_or_else(|| {
            panic!("Failed to create log directory.");
        });

        let (tx, rx) = mpsc::channel::<String>(LOG_CHANNEL_CAPACITY);
        let stats = Arc::new(LoggerWriterStats::new(LOG_CHANNEL_CAPACITY));

        // 使用專屬 thread 與 runtime，讓 logger worker 不受測試或呼叫端 tokio runtime 生命週期影響。
        let path = log_path.display().to_string();
        let worker_stats = Arc::clone(&stats);
        thread::spawn(move || {
            let rt = Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|e| panic!("Failed to build logger runtime: {e}"));
            rt.block_on(Self::process_messages(rx, path, worker_stats, seq_level));
        });

        AsyncLogWriter { sender: tx, stats }
    }

    /// 背景處理日誌 queue，負責寫檔與送出 Seq。
    ///
    /// 檔案寫入會累積成小批次降低 IO 次數；Seq 發送只 enqueue 到背景
    /// sender，避免此 worker 等待網路回應。
    async fn process_messages(
        mut rx: Receiver<String>,
        log_path: String,
        stats: Arc<LoggerWriterStats>,
        seq_level: SeqLogLevel,
    ) {
        let mut msg = String::with_capacity(2048);
        let mut rotate = Rotate::new(log_path);

        while let Some(message) = rx.recv().await {
            decrement_atomic_usize(&stats.queued_messages);
            stats.processed_messages.fetch_add(1, Ordering::Relaxed);
            let now = Local::now();

            // Seq 發送只做 best-effort enqueue；本地寫檔仍是主要保底。
            forward_to_seq(seq_level, &message);

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

/// 初始化 Seq 日誌轉送。
///
/// `server_url` 空白時代表停用 Seq；`api_key` 空白時仍會送出事件，但不附帶
/// `X-Seq-ApiKey`。此函式應在 `.env` 載入後呼叫，確保環境變數覆蓋已生效。
pub async fn init_seq<S, K>(server_url: S, api_key: K)
where
    S: AsRef<str>,
    K: AsRef<str>,
{
    let server_url = server_url.as_ref().trim();
    let api_key = api_key.as_ref().trim();

    if server_url.is_empty() {
        return;
    }

    if SEQ_SENDER.get().is_some() {
        SEQ_LOGGING_ENABLED.store(true, Ordering::Relaxed);
        return;
    }

    let endpoint = server_url.trim_end_matches('/').to_string();
    let api_key = (!api_key.is_empty()).then(|| api_key.to_string());
    let (tx, rx) = mpsc::channel::<SeqEvent>(SEQ_CHANNEL_CAPACITY);

    match SEQ_SENDER.set(tx) {
        Ok(()) => {
            SEQ_LOGGING_ENABLED.store(true, Ordering::Relaxed);
            spawn_seq_worker(rx, endpoint, api_key);
            info_console(format!("Seq logging enabled: {}", server_url));
        }
        Err(_) => {
            SEQ_LOGGING_ENABLED.store(true, Ordering::Relaxed);
        }
    }
}

/// 啟動 Seq 背景發送 worker。
///
/// 使用專屬 thread 與 tokio runtime，讓 Seq 發送不依賴呼叫端 runtime 的生命週期。
fn spawn_seq_worker(mut rx: Receiver<SeqEvent>, endpoint: String, api_key: Option<String>) {
    thread::spawn(move || {
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| panic!("Failed to build Seq logger runtime: {e}"));

        rt.block_on(async move {
            ensure_rustls_crypto_provider();
            let client = match Client::builder().build() {
                Ok(client) => client,
                Err(why) => {
                    error_console(format!("Failed to build Seq HTTP client: {:?}", why));
                    return;
                }
            };

            process_seq_events(&client, &endpoint, api_key.as_deref(), &mut rx).await;
        });
    });
}

/// 批次處理 Seq 事件 queue。
///
/// 若 Seq 暫時不可用，事件會在本次送出失敗後丟棄，避免背景 queue 以外再累積
/// 無界記憶體；檔案日誌仍保留完整資料。
async fn process_seq_events(
    client: &Client,
    endpoint: &str,
    api_key: Option<&str>,
    rx: &mut Receiver<SeqEvent>,
) {
    let mut buf = Vec::with_capacity(SEQ_BATCH_EVENT_LIMIT);
    let mut ticker = time::interval(Duration::from_millis(SEQ_FLUSH_INTERVAL_MS));

    loop {
        tokio::select! {
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        buf.push(event);
                        if buf.len() >= SEQ_BATCH_EVENT_LIMIT {
                            flush_seq_events(client, endpoint, api_key, &mut buf).await;
                        }
                    }
                    None => {
                        flush_seq_events(client, endpoint, api_key, &mut buf).await;
                        break;
                    }
                }
            }
            _ = ticker.tick() => {
                flush_seq_events(client, endpoint, api_key, &mut buf).await;
            }
        }
    }
}

/// 將目前累積的事件以 CLEF 格式送到 Seq。
///
/// CLEF 每列是一筆 JSON 事件，使用 `/api/events/raw?clef` endpoint。
async fn flush_seq_events(
    client: &Client,
    endpoint: &str,
    api_key: Option<&str>,
    buf: &mut Vec<SeqEvent>,
) {
    if buf.is_empty() {
        return;
    }

    let payload = buf
        .iter()
        .filter_map(|event| serde_json::to_string(event).ok())
        .collect::<Vec<_>>()
        .join("\n");

    buf.clear();

    if payload.is_empty() {
        return;
    }

    let mut request = client
        .post(format!("{}/api/events/raw?clef", endpoint))
        .header("Content-Type", "application/vnd.serilog.clef")
        .body(payload);

    if let Some(api_key) = api_key {
        request = request.header("X-Seq-ApiKey", api_key);
    }

    match request.send().await {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            error_console(format!(
                "Seq logging failed with status {}",
                response.status()
            ));
        }
        Err(why) => {
            error_console(format!("Seq logging request failed: {:?}", why));
        }
    }
}

/// 將一筆既有檔案日誌轉送到 Seq。
///
/// 事件會以 CLEF 格式進入背景 queue；queue 滿時直接丟棄，避免日誌流量反向拖慢
/// 應用程式主流程。
fn forward_to_seq(level: SeqLogLevel, message: &str) {
    if !SEQ_LOGGING_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    if let Some(sender) = SEQ_SENDER.get() {
        let event = SeqEvent {
            timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true),
            message_template: message.to_string(),
            level: level.as_seq_level(),
            service: SEQ_SERVICE_NAME,
            rust_log_level: level.as_rust_level(),
            logger: "core::logging",
        };

        let _ = sender.try_send(event);
    }
}

/// 將累積的日誌文字寫入輪轉檔案。
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
