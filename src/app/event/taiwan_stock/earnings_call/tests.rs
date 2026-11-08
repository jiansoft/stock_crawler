//! 法說會通知的測試。

use super::*;

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

fn call(call_id: &str, date: NaiveDate) -> EarningsCall {
    EarningsCall {
        call_id: call_id.to_string(),
        date,
        title: "台積電 2026-10-05 法說會".to_string(),
        has_transcript: true,
    }
}

/// 通知紀錄寫入 Redis 後讀得回來；測試用假場次，結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn notified_record_round_trips_through_redis() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 notified_record_round_trips_through_redis：無 Redis 連線");
        return;
    }
    let call_id = "TEST_79988.TW_1970-01-01";
    let key = notified_key(call_id);
    let _ = CLIENT.delete(&key).await;

    assert!(!is_notified(call_id).await, "尚未記錄前應為未通知");
    mark_notified(call_id).await;
    assert!(is_notified(call_id).await, "記錄後應為已通知");

    CLIENT.delete(&key).await.expect("清除測試紀錄");
}

/// 只看回看天數內（含今天）的場次，未來日期與太舊的都不看。
#[test]
fn recent_calls_keeps_the_lookback_window() {
    let calls = vec![
        call("future", day(10, 6)),
        call("today", day(10, 5)),
        call("edge", day(9, 28)),
        call("old", day(9, 27)),
    ];
    let ids: Vec<&str> = recent_calls(&calls, day(10, 5))
        .into_iter()
        .map(|call| call.call_id.as_str())
        .collect();
    assert_eq!(ids, vec!["today", "edge"]);
}

#[test]
fn clean_section_strips_bold_markers_and_truncates() {
    assert_eq!(
        clean_section("  **Q: 展望**\nA: 樂觀  "),
        "Q: 展望\nA: 樂觀"
    );
    let long = "字".repeat(SECTION_MAX_CHARS + 10);
    let cleaned = clean_section(&long);
    assert_eq!(cleaned.chars().count(), SECTION_MAX_CHARS + 1);
    assert!(cleaned.ends_with('…'));
}

/// 訊息包含標頭、三個段落與 BigGo 連結，特殊字元依 MarkdownV2 跳脫，空段落略過。
#[test]
fn build_message_formats_sections_and_link() {
    let detail = EarningsCallDetail {
        call_id: "TW_2330.TW_2026-07-16".to_string(),
        date: Some(day(7, 16)),
        title: "台積電 2026-07-16 法說會".to_string(),
        summary: "營收 402 億美元（季增 11.3%）".to_string(),
        operations: "不會出現".to_string(),
        outlook: String::new(),
        qa_highlights: "**Q: 資本支出?**\nA: 上修。".to_string(),
    };
    let message = build_message(
        "2330",
        "台積電",
        &call("TW_2330.TW_2026-07-16", day(7, 16)),
        &detail,
    );
    assert!(message.starts_with("持股法說會︰2330 台積電（2026\\-07\\-16）\n"));
    assert!(message.contains("【摘要】\n營收 402 億美元（季增 11\\.3%）"));
    assert!(!message.contains("【展望】"), "空段落不輸出");
    assert!(!message.contains("不會出現"), "營運重點不放進通知");
    assert!(message.contains("【Q&A 重點】\nQ: 資本支出?\nA: 上修。"));
    assert!(message.ends_with(
        "完整內容︰[BigGo](https://finance\\.biggo\\.com\\.tw/quote/2330\\.TW/earnings\\-call/TW\\_2330\\.TW\\_2026\\-07\\-16)"
    ));
}

/// 單場處理的四種結果：已通知、內容抓取失敗、摘要尚未產生、送出並記錄（mock BigGo＋Redis）。
/// 用 1970 年的假場次，結束時刪除 Redis 紀錄。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn notify_covers_every_outcome() {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 notify_covers_every_outcome：無 Redis 連線");
        return;
    }
    let ids = [
        "TEST_79989.TW_1970-01-01",
        "TEST_79989.TW_1970-01-02",
        "TEST_79989.TW_1970-01-03",
        "TEST_79989.TW_1970-01-04",
    ];
    for id in ids {
        let _ = CLIENT.delete(&notified_key(id)).await;
    }
    let [already, failed, pending, ready] = ids;

    let server = MockServer::start().await;
    let detail = |call_id: &str, summary: &str| {
        serde_json::json!({
            "result": true,
            "data": { "call_id": call_id, "date": "1970-01-04", "title": "測試法說會", "summary": summary }
        })
    };
    let route = "/api/v1/finance/stock/earnings-calls/detail";
    Mock::given(method("GET"))
        .and(path(route))
        .and(query_param("call_id", failed))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(route))
        .and(query_param("call_id", pending))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail(pending, " ")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(route))
        .and(query_param("call_id", ready))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail(ready, "營收成長")))
        .expect(1)
        .mount(&server)
        .await;

    mark_notified(already).await;
    let outcomes = crate::core::util::http::mock_origin::with_origins(
        &[("https://api.biggo.com", &server.uri())],
        async {
            let mut outcomes = Vec::new();
            for id in ids {
                outcomes.push(notify("79989", &call(id, day(1, 4))).await);
            }
            outcomes
        },
    )
    .await;
    let ready_marked = is_notified(ready).await;
    let pending_marked = is_notified(pending).await;
    for id in ids {
        let _ = CLIENT.delete(&notified_key(id)).await;
    }

    assert!(matches!(
        outcomes.as_slice(),
        [
            Outcome::AlreadyNotified,
            Outcome::Failed,
            Outcome::Pending,
            Outcome::Notified
        ]
    ));
    assert!(ready_marked, "送出後要記錄已通知");
    assert!(!pending_marked, "摘要未產生不能記錄，下次還要再試");
}

#[test]
fn notified_key_uses_the_call_id() {
    assert_eq!(
        notified_key("TW_2330.TW_2026-07-16"),
        "biggo:earnings_call:notified:TW_2330.TW_2026-07-16"
    );
}

/// 以臨時資料庫的假持股、mock BigGo 跑排程入口：回看期間內、摘要已產生的場次只通知一次。
///
/// 假持股代號 799797（全數字才算普通股）、會員 979797；Telegram 導向 mock server，測試也沒有
/// 註冊告警管道，不會真的送出訊息。結束時刪除持股與 Redis 已通知紀錄。
mod execute_flow {
    use chrono::Days;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;
    use crate::infra::database;

    const SYMBOL: &str = "799797";
    const MEMBER: i64 = 979_797;

    fn call_id(date: NaiveDate) -> String {
        format!("TW_{SYMBOL}.TW_{date}")
    }

    async fn cleanup(recent: &str) {
        sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
            .bind(MEMBER)
            .execute(database::get_connection())
            .await
            .expect("清除測試持股");
        let _ = CLIENT.delete(&notified_key(recent)).await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn execute_notifies_recent_ready_call_once() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 execute_notifies_recent_ready_call_once：無資料庫連接");
            return;
        }
        let today = Local::now().date_naive();
        let recent = call_id(today);
        let stale = call_id(today - Days::new(30));
        cleanup(&recent).await;
        sqlx::query(
            "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, created_time, date) \
             VALUES ($1, $2, 1000, '1999-03-10 00:00:00+08', '1999-03-10')",
        )
        .bind(MEMBER)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("插入測試持股");

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/finance/stock/{SYMBOL}.TW/earnings-calls/list")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": true,
                "data": {"list": [
                    {"call_id": recent, "date": today.to_string(), "title": "測試法說會", "has_transcript": true},
                    {"call_id": stale, "date": (today - Days::new(30)).to_string(), "title": "舊場次", "has_transcript": true}
                ]}
            })))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/finance/stock/earnings-calls/detail"))
            .and(query_param("call_id", recent.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": true,
                "data": {"call_id": recent, "date": today.to_string(), "title": "測試法說會",
                         "summary": "營收創新高", "outlook": "審慎樂觀", "qa_highlights": ""}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let uri = server.uri();

        let (first, second) = with_origins(
            &[
                ("https://api.biggo.com", &uri),
                ("https://api.telegram.org", &uri),
            ],
            async { (execute().await, execute().await) },
        )
        .await;
        let notified = is_notified(&recent).await;
        let stale_notified = is_notified(&stale).await;
        cleanup(&recent).await;

        first.expect("第一次通知");
        second.expect("第二次略過已通知");
        assert!(notified, "送出後記錄已通知");
        assert!(!stale_notified, "回看期間外的場次不處理");
        // 內容只抓一次：第二次執行時已通知，不再抓內容（MockServer 結束時驗證次數）。
    }
}
