//! tracing 與輪轉檔案日誌的橋接：[`FileLogLayer`] 把事件寫進 [`super::LOGGER`] 並轉送 Seq。

use std::{collections::HashMap, fmt::Write as _};

use super::{
    LOGGER,
    seq::{SeqLogLevel, forward_to_seq},
};

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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

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
}
