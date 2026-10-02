//! Seq 日誌轉送：把 tracing 事件以 CLEF 格式批次送到 Seq。
//!
//! 由 [`super::layer::FileLogLayer`] 呼叫 [`forward_to_seq`]；背景 worker 使用專屬 thread 與
//! runtime，Seq 不可用時只丟棄事件，不影響檔案日誌。

use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

use chrono::{SecondsFormat, Utc};
use once_cell::sync::OnceCell;
use reqwest::Client;
use serde::Serialize;
use tokio::{
    runtime::Builder,
    sync::mpsc::{self, Receiver, Sender},
    time::{self, Duration},
};

use super::{error_console, info_console};
use crate::core::util::ensure_rustls_crypto_provider;

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
pub(super) enum SeqLogLevel {
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
    pub(super) fn as_rust_level(self) -> &'static str {
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
    pub(super) fn as_seq_level(self) -> &'static str {
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

/// 判斷 Seq 端點是否足以安全攜帶 `X-Seq-ApiKey`。
///
/// API key 是以 HTTP 標頭送出的，走明文 HTTP 時任何能看到封包的人都能取得它。
/// 兩種情況視為安全：
///
/// - `https://`：金鑰在 TLS 內。
/// - 指向 loopback 的 `http://`：封包不會離開本機，沒有被竊聽的空間。
///   本機跑 Seq container 是常見的開發組態，不該被擋。
///
/// 其餘（例如 `.env` 範例中的 `http://192.168.111.224:5341` 這種區域網路位址）
/// 一律視為不安全 —— 區域網路同樣可能被側錄。
fn seq_endpoint_protects_api_key(endpoint: &str) -> bool {
    if endpoint.starts_with("https://") {
        return true;
    }

    let Some(rest) = endpoint.strip_prefix("http://") else {
        // 既非 http 也非 https（設定錯誤或未知 scheme），保守起見視為不安全。
        return false;
    };

    // 取出主機部分：去掉路徑、userinfo 與連接埠。
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = match host.strip_prefix('[') {
        // IPv6 字面值形如 [::1]:5341。
        Some(after_bracket) => after_bracket.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };

    host.eq_ignore_ascii_case("localhost")
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// 初始化 Seq 日誌轉送。
///
/// `server_url` 空白時代表停用 Seq；`api_key` 空白時仍會送出事件，但不附帶
/// `X-Seq-ApiKey`。此函式應在 `.env` 載入後呼叫，確保環境變數覆蓋已生效。
///
/// # 安全性
///
/// 端點若不是 HTTPS 也不是 loopback，**API key 會被捨棄**（事件照常送出，
/// 只是不附帶金鑰），並在 console 留下警告。這道防護是為了避免把金鑰以明文
/// 送上網路 —— Seq 的轉送不經過 [`crate::core::util::http`]，
/// 沒有那一層的保護。
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
    let api_key = match (!api_key.is_empty()).then(|| api_key.to_string()) {
        // 明文端點一律不附帶金鑰：寧可 Seq 拒收（會在送出端記錄狀態碼），
        // 也不要把金鑰攤在網路上。
        Some(_) if !seq_endpoint_protects_api_key(&endpoint) => {
            error_console(format!(
                "Seq endpoint {endpoint} is not HTTPS and not loopback; \
                 X-Seq-ApiKey will NOT be sent to avoid transmitting it in cleartext"
            ));
            None
        }
        other => other,
    };
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
pub(super) fn forward_to_seq(
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// HTTPS 端點可安全攜帶 API key。
    #[test]
    fn seq_endpoint_accepts_https() {
        assert!(seq_endpoint_protects_api_key("https://seq.example.com"));
        assert!(seq_endpoint_protects_api_key(
            "https://192.168.111.224:5341"
        ));
    }

    /// loopback 的明文端點可接受：封包不會離開本機。
    #[test]
    fn seq_endpoint_accepts_plaintext_loopback() {
        for endpoint in [
            "http://localhost:5341",
            "http://LOCALHOST",
            "http://127.0.0.1:5341",
            "http://127.1.2.3",
            "http://[::1]:5341",
        ] {
            assert!(
                seq_endpoint_protects_api_key(endpoint),
                "{endpoint} 應被視為安全"
            );
        }
    }

    /// 明文的非 loopback 端點必須被判定為不安全。
    ///
    /// `.env` 範例中的 `http://192.168.111.224:5341` 正是這一類 ——
    /// 區域網路同樣可能被側錄。
    #[test]
    fn seq_endpoint_rejects_plaintext_remote() {
        for endpoint in [
            "http://192.168.111.224:5341",
            "http://seq.example.com",
            "http://user:pass@seq.example.com:5341",
            "http://[2001:db8::1]:5341",
            "ftp://seq.example.com",
            "seq.example.com",
            "",
        ] {
            assert!(
                !seq_endpoint_protects_api_key(endpoint),
                "{endpoint} 不應被視為安全"
            );
        }
    }

    /// 主機名稱以 localhost 開頭但實為他站者，不得誤判為 loopback。
    #[test]
    fn seq_endpoint_rejects_lookalike_hosts() {
        assert!(!seq_endpoint_protects_api_key("http://localhost.evil.com"));
        assert!(!seq_endpoint_protects_api_key("http://notlocalhost"));
    }
}
