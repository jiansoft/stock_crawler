//! tracing 與輪轉檔案日誌的橋接：[`FileLogLayer`] 把事件寫進 [`super::LOGGER`] 並轉送 Seq的測試。

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

/// `db.statement` 在檔案日誌中壓縮空白並截斷，其他欄位照原樣輸出。
#[test]
fn file_field_truncates_db_statement_only() {
    let sql = format!("\n\n  SELECT 1\n   FROM t {}", "x".repeat(400));
    let mut line = String::new();
    write_file_field(&mut line, "db.statement", &serde_json::Value::from(sql));
    assert!(line.starts_with("\"SELECT 1 FROM t xxx"), "{line}");
    assert!(line.contains("…（共 416 字）"), "{line}");
    assert!(!line.contains("\\n"), "{line}");

    let mut short = String::new();
    write_file_field(
        &mut short,
        "db.statement",
        &serde_json::Value::from("SELECT\n 1"),
    );
    assert_eq!(short, "\"SELECT 1\"");

    let mut other = String::new();
    write_file_field(&mut other, "summary", &serde_json::Value::from("a\nb"));
    assert_eq!(other, "\"a\\nb\"");

    let mut number = String::new();
    write_file_field(&mut number, "elapsed_ms", &serde_json::Value::from(12));
    assert_eq!(number, "12");
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
    assert!(DEFAULT_FILE_LOG_DIRECTIVES.contains("tokio_cron_scheduler=warn"));
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
    assert!(
        filter.contains("tokio_cron_scheduler=warn"),
        "實際過濾器：{filter}"
    );
}
