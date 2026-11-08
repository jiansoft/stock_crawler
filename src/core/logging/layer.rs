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
/// - `tokio_cron_scheduler=warn`：排程套件每註冊一個工作就以 `info!` 印出沒有工作名稱的
///   「Uninited」「Job creator created」，啟動時一次幾十行；工作名稱與週期已由
///   `app::scheduler` 的「註冊排程任務」記錄，這兩行沒有資訊量。保留套件的 warn／error。
const DEFAULT_FILE_LOG_DIRECTIVES: &str =
    "info,html5ever=off,rustls::msgs::handshake=error,tokio_cron_scheduler=warn";

/// 檔案日誌（`FileLogLayer`）的等級過濾器。
///
/// 由環境變數 `FILE_LOG_LEVEL` 控制，未設定時採用 [`DEFAULT_FILE_LOG_DIRECTIVES`]，
/// 避免生產環境把大量 DEBUG/TRACE 或第三方套件雜訊寫入磁碟導致日誌檔暴增
/// （曾發生單一 `*_debug.log` 長到 10G、根目錄塞爆）。
///
/// - 不設定時：只落 `INFO` 以上，並關閉 `html5ever` 雜訊。
/// - 臨時除錯：`FILE_LOG_LEVEL=debug`（或更細的 `info,stock_crawler::app::event::trace=debug`）。
///   注意自訂時若仍想壓掉第三方雜訊，記得自行附帶 `,html5ever=off,rustls::msgs::handshake=error,tokio_cron_scheduler=warn`。
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
        super::slow_query::observe(target, level, &mut collector.extra);

        // 建立檔案日誌行：message 後追加所有結構化欄位（key=val）。
        let log_line = {
            let mut line = collector.message.clone();
            for (k, v) in &collector.extra {
                let _ = write!(line, " {k}=");
                write_file_field(&mut line, k, v);
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

/// 檔案日誌中 `db.statement` 最多保留的字元數。
///
/// sqlx 的慢查詢事件會帶整段 SQL（估價重建約 8 KB），檔案日誌一行就被撐爆；
/// 開頭已足以辨識是哪支查詢（另有 `summary` 欄位），完整 SQL 仍會送到 Seq。
const FILE_DB_STATEMENT_MAX_CHARS: usize = 200;

/// 把單一結構化欄位寫進檔案日誌行；`db.statement` 壓縮空白並截斷。
fn write_file_field(line: &mut String, key: &str, value: &serde_json::Value) {
    let serde_json::Value::String(text) = value else {
        let _ = write!(line, "{value}");
        return;
    };
    if key != "db.statement" {
        let _ = write!(line, "{value}");
        return;
    }

    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let total = compact.chars().count();
    let shown = if total > FILE_DB_STATEMENT_MAX_CHARS {
        let head = compact
            .chars()
            .take(FILE_DB_STATEMENT_MAX_CHARS)
            .collect::<String>();
        format!("{head}…（共 {total} 字）")
    } else {
        compact
    };
    let _ = write!(line, "{}", serde_json::Value::String(shown));
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
mod tests;
