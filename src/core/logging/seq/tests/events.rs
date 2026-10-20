//! 日誌等級對照、CLEF 序列化，以及停用或未設定伺服器時的行為。

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
