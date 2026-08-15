//! 非同步檔案與主控台日誌工具。

use std::{
    collections::HashMap,
    fmt::Write as _,
    fs::{self},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
};

use chrono::{Local, SecondsFormat, Utc, format::DelayedFormat};
use once_cell::sync::{Lazy, OnceCell};
use reqwest::Client;
use serde::Serialize;
use tokio::{
    runtime::Builder,
    sync::mpsc::{self, Receiver, Sender, error::TrySendError},
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
/// `fields` 以 `#[serde(flatten)]` 展開為頂層 JSON 屬性，讓 Seq 可直接搜尋結構化欄位。
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
    /// 事件來源模組路徑（取自 `tracing::Metadata::target()`）。
    #[serde(rename = "Logger")]
    logger: String,
    /// tracing 事件附加的結構化欄位（如 `stock_symbol`、`elapsed_ms` 等）。
    /// 展開為頂層 JSON 屬性，讓 Seq filter 可直接用 `stock_symbol = '2330'` 查詢。
    #[serde(flatten)]
    fields: HashMap<String, serde_json::Value>,
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

/// 將 tracing 事件轉送到 Seq（結構化 CLEF 格式）。
///
/// 由 `FileLogLayer::on_event` 直接呼叫，攜帶完整的結構化欄位。
/// 事件進入背景 queue；queue 滿時直接丟棄，避免日誌流量拖慢主流程。
fn forward_to_seq(
    level: SeqLogLevel,
    message: &str,
    fields: HashMap<String, serde_json::Value>,
    target: &str,
) {
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
            logger: target.to_string(),
            fields,
        };

        let _ = sender.try_send(event);
    }
}

/// 檔案日誌（`FileLogLayer`）的預設過濾指令。
///
/// - `info`：基準等級，只落 `INFO` 以上（`info` / `warn` / `error`）。
/// - `html5ever=off`：關閉第三方 HTML 解析套件 `html5ever` 的日誌。它在解析畸形網頁時
///   會以 `warn!` 噴出大量「foster parenting not implemented」，屬無害雜訊，且因為是
///   `warn` 等級，單靠 `info` 基準擋不掉，必須針對 target 關閉。
/// - `rustls::msgs::handshake=error`：以 IP（而非網域）直連 HTTPS 時，rustls 會以 `warn!`
///   噴出「Illegal SNI extension: ignoring IP address presented as hostname」。依 RFC 6066
///   SNI 僅能放主機名稱，rustls 只是忽略該 IP 後照常握手，屬無害雜訊。降到 `error`
///   可壓掉此警告，同時保留真正的 TLS 錯誤。
const DEFAULT_FILE_LOG_DIRECTIVES: &str = "info,html5ever=off,rustls::msgs::handshake=error";

/// 檔案日誌（`FileLogLayer`）的等級過濾器。
///
/// 由環境變數 `FILE_LOG_LEVEL` 控制，未設定時採用 [`DEFAULT_FILE_LOG_DIRECTIVES`]，
/// 避免生產環境把大量 DEBUG/TRACE 或第三方套件雜訊寫入磁碟導致日誌檔暴增
/// （曾發生單一 `*_debug.log` 長到 10G、根目錄塞爆）。
///
/// - 不設定時：只落 `INFO` 以上，並關閉 `html5ever` 雜訊。
/// - 臨時除錯：`FILE_LOG_LEVEL=debug`（或更細的 `info,stock_crawler::app::event::trace=debug`）。
///   注意自訂時若仍想壓掉第三方雜訊，記得自行附帶 `,html5ever=off,rustls::msgs::handshake=error`。
///
/// 此過濾器只作用於檔案日誌層；stdout 的 fmt 層仍由 `RUST_LOG` 獨立控制。
pub fn file_log_env_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_env("FILE_LOG_LEVEL")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_FILE_LOG_DIRECTIVES))
}

/// tracing `Layer`，將 tracing 事件路由至既有輪轉檔案 `LOGGER` 並轉送 Seq。
///
/// - 安裝後所有 `tracing::*!()` 事件都寫入輪轉日誌。
/// - `message` 以外的附加欄位（如 `stock_symbol`、`elapsed_ms`）以 `key=val` 格式附在
///   檔案日誌行末，並作為 CLEF 頂層屬性送到 Seq，讓 Seq 可用結構化查詢。
/// - Level 映射：ERROR → error_writer、WARN → warn_writer、INFO → info_writer、其餘 → debug_writer。
pub struct FileLogLayer;

impl<S: tracing::Subscriber> tracing_subscriber::layer::Layer<S> for FileLogLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut collector = FieldCollector::default();
        event.record(&mut collector);
        if collector.message.is_empty() {
            return;
        }

        let target = event.metadata().target();
        let level = *event.metadata().level();

        // 建立檔案日誌行：message 後追加所有結構化欄位（key=val）。
        let log_line = {
            let mut line = collector.message.clone();
            for (k, v) in &collector.extra {
                let _ = write!(line, " {k}={v}");
            }
            line
        };

        // 轉換為 Seq 結構化欄位 map。
        let fields: HashMap<String, serde_json::Value> = collector.extra.into_iter().collect();

        match level {
            tracing::Level::ERROR => {
                LOGGER.error(log_line);
                forward_to_seq(SeqLogLevel::Error, &collector.message, fields, target);
            }
            tracing::Level::WARN => {
                LOGGER.warn(log_line);
                forward_to_seq(SeqLogLevel::Warn, &collector.message, fields, target);
            }
            tracing::Level::INFO => {
                LOGGER.info(log_line);
                forward_to_seq(SeqLogLevel::Info, &collector.message, fields, target);
            }
            _ => {
                LOGGER.debug(log_line);
                forward_to_seq(SeqLogLevel::Debug, &collector.message, fields, target);
            }
        }
    }
}

/// 從 tracing 事件收集所有欄位的訪客型別。
///
/// - `message` 欄位存入 `message`。
/// - 其餘欄位（結構化屬性）以 `(name, serde_json::Value)` 收集到 `extra`。
#[derive(Default)]
struct FieldCollector {
    message: String,
    extra: Vec<(String, serde_json::Value)>,
}

impl tracing::field::Visit for FieldCollector {
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.extra
            .push((field.name().to_string(), serde_json::Value::from(value)));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.extra
            .push((field.name().to_string(), serde_json::Value::from(value)));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.extra
            .push((field.name().to_string(), serde_json::Value::from(value)));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.extra
            .push((field.name().to_string(), serde_json::Value::from(value)));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.extra
                .push((field.name().to_string(), serde_json::Value::from(value)));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // format_args! 實作 Debug 時輸出即是格式化結果（無額外引號）。
            let _ = write!(self.message, "{value:?}");
        } else {
            self.extra.push((
                field.name().to_string(),
                serde_json::Value::from(format!("{value:?}")),
            ));
        }
    }
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
    use std::sync::Mutex;

    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    /// 單筆擷取結果：（訊息, 結構化欄位）。
    type CapturedEvent = (String, Vec<(String, serde_json::Value)>);

    /// 測試用的擷取層：把每筆事件交給 [`FieldCollector`] 解析後留存。
    ///
    /// 刻意不直接安裝 [`FileLogLayer`]：那會喚醒全域 `LOGGER` 並真的寫檔，
    /// 測試只想驗證「事件 → 訊息 + 結構化欄位」這段轉換。
    struct CaptureLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S: tracing::Subscriber> tracing_subscriber::layer::Layer<S> for CaptureLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut collector = FieldCollector::default();
            event.record(&mut collector);
            if let Ok(mut events) = self.events.lock() {
                events.push((collector.message, collector.extra));
            }
        }
    }

    /// 在只裝了 [`CaptureLayer`] 的 subscriber 下執行 `emit`，回傳擷取到的事件。
    fn capture(emit: impl FnOnce()) -> Vec<CapturedEvent> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(CaptureLayer {
            events: Arc::clone(&events),
        });
        tracing::subscriber::with_default(subscriber, emit);

        events.lock().expect("擷取結果應可取得").clone()
    }

    /// 專案自用等級名稱與 Seq/Serilog 等級名稱各自獨立，不可混用。
    #[test]
    fn seq_log_level_maps_to_both_naming_schemes() {
        for (level, rust, seq) in [
            (SeqLogLevel::Info, "Info", "Information"),
            (SeqLogLevel::Warn, "Warn", "Warning"),
            (SeqLogLevel::Error, "Error", "Error"),
            (SeqLogLevel::Debug, "Debug", "Debug"),
        ] {
            assert_eq!(level.as_rust_level(), rust);
            assert_eq!(level.as_seq_level(), seq);
        }
    }

    /// `message` 欄位進 message，其餘欄位保持型別進 extra。
    #[test]
    fn field_collector_separates_message_from_structured_fields() {
        let captured = capture(|| {
            tracing::info!(
                stock_symbol = "2330",
                elapsed_ms = 12_i64,
                rows = 3_u64,
                ratio = 1.5_f64,
                cached = true,
                "個股月行情回補完成"
            );
        });

        assert_eq!(captured.len(), 1);
        let (message, extra) = &captured[0];
        assert_eq!(message, "個股月行情回補完成");

        let fields: HashMap<&str, &serde_json::Value> =
            extra.iter().map(|(k, v)| (k.as_str(), v)).collect();
        // 數值欄位必須保留 JSON 原生型別，Seq 才能用 `elapsed_ms > 10` 之類的條件查詢。
        assert_eq!(fields["stock_symbol"], &serde_json::json!("2330"));
        assert_eq!(fields["elapsed_ms"], &serde_json::json!(12));
        assert_eq!(fields["rows"], &serde_json::json!(3));
        assert_eq!(fields["ratio"], &serde_json::json!(1.5));
        assert_eq!(fields["cached"], &serde_json::json!(true));
    }

    /// 格式化訊息（`format_args!`）走 `record_debug`，不可留下多餘引號。
    #[test]
    fn formatted_message_is_collected_without_debug_quotes() {
        let captured = capture(|| {
            let symbol = "0050";
            tracing::warn!("個股月行情抓取失敗：{}", symbol);
        });

        assert_eq!(captured[0].0, "個股月行情抓取失敗：0050");
        assert!(captured[0].1.is_empty());
    }

    /// 無 message 的事件會在 `FileLogLayer` 被略過，收集結果應為空字串。
    #[test]
    fn event_without_message_collects_empty_message() {
        let captured = capture(|| {
            tracing::info!(stock_symbol = "2330");
        });

        assert!(captured[0].0.is_empty());
        assert_eq!(captured[0].1.len(), 1);
    }

    /// CLEF 事件必須使用 Seq 規定的 `@t`/`@mt`/`@l` 欄位名，並把結構化欄位攤平到頂層。
    #[test]
    fn seq_event_serializes_to_clef_shape() {
        let event = SeqEvent {
            timestamp: "2026-08-15T01:02:03.000000Z".to_string(),
            message_template: "個股月行情回補完成".to_string(),
            level: SeqLogLevel::Info.as_seq_level(),
            service: SEQ_SERVICE_NAME,
            rust_log_level: SeqLogLevel::Info.as_rust_level(),
            logger: "stock_crawler::app::backfill".to_string(),
            fields: HashMap::from([("stock_symbol".to_string(), serde_json::json!("2330"))]),
        };

        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&event).expect("序列化應成功"))
                .expect("結果應為合法 JSON");

        assert_eq!(json["@t"], "2026-08-15T01:02:03.000000Z");
        assert_eq!(json["@mt"], "個股月行情回補完成");
        assert_eq!(json["@l"], "Information");
        assert_eq!(json["RustLogLevel"], "Info");
        assert_eq!(json["service"], SEQ_SERVICE_NAME);
        assert_eq!(json["Logger"], "stock_crawler::app::backfill");
        // flatten：結構化欄位是頂層屬性，不是巢狀的 "fields" 物件。
        assert_eq!(json["stock_symbol"], "2330");
        assert!(json.get("fields").is_none());
    }

    /// 未啟用 Seq 時 `forward_to_seq` 必須是 no-op，不可 panic。
    #[test]
    fn forward_to_seq_is_a_noop_when_disabled() {
        let was_enabled = SEQ_LOGGING_ENABLED.swap(false, Ordering::Relaxed);
        forward_to_seq(SeqLogLevel::Info, "訊息", HashMap::new(), "test");
        SEQ_LOGGING_ENABLED.store(was_enabled, Ordering::Relaxed);
    }

    /// 空的 server_url 代表停用 Seq，不得建立 sender。
    #[tokio::test]
    async fn init_seq_ignores_a_blank_server_url() {
        let was_enabled = SEQ_LOGGING_ENABLED.load(Ordering::Relaxed);
        init_seq("   ", "key").await;
        assert_eq!(SEQ_LOGGING_ENABLED.load(Ordering::Relaxed), was_enabled);
        assert!(SEQ_SENDER.get().is_none() || was_enabled);
    }

    /// 預設過濾指令必須壓掉已知的第三方雜訊來源。
    #[test]
    fn default_file_log_directives_silence_known_noise() {
        assert!(DEFAULT_FILE_LOG_DIRECTIVES.starts_with("info"));
        // html5ever 的雜訊是 warn 等級，只靠 info 基準擋不掉，必須逐 target 關閉。
        assert!(DEFAULT_FILE_LOG_DIRECTIVES.contains("html5ever=off"));
        assert!(DEFAULT_FILE_LOG_DIRECTIVES.contains("rustls::msgs::handshake=error"));
    }

    /// 未設定 `FILE_LOG_LEVEL` 時，過濾器應回落到預設指令。
    #[test]
    fn file_log_env_filter_falls_back_to_the_default_directives() {
        if std::env::var("FILE_LOG_LEVEL").is_ok() {
            println!(
                "跳過 file_log_env_filter_falls_back_to_the_default_directives：環境已設定 FILE_LOG_LEVEL"
            );
            return;
        }

        let filter = file_log_env_filter().to_string();
        assert!(filter.contains("html5ever=off"), "實際過濾器：{filter}");
    }

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
