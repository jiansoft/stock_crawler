//! 本週 HTTP 異常：解析 warn／error 日誌的 http 事件、依網域加總、排名與門檻警示。

use super::*;
use crate::app::event::taiwan_stock::data_health::http_errors::{
    HostIssues, http_issue_line, parse_http_line, weekly_http_issues,
};

#[test]
fn parse_http_line_classifies_events() {
    let slow = r#"2026-10-26 05:37:10.3 http.slow url="https://www.twse.com.tw/rwd/zh/exRight/TWT49U?response=json" method="GET" status=200 elapsed_ms=10250"#;
    assert_eq!(
        parse_http_line(slow),
        Some((
            "www.twse.com.tw".to_owned(),
            HostIssues {
                slow: 1,
                ..HostIssues::default()
            }
        ))
    );
    let fail_fast = r#"http.failed.fail_fast params=[is_check=1] url="https://pchome.megatime.com.tw/stock/sid1303.html" attempt=1"#;
    assert_eq!(parse_http_line(fail_fast).map(|(_, i)| i.failed), Some(1));
    let forbidden = r#"http.done url="https://api.biggo.com/api/v1/finance/stock" method="GET" status=403 elapsed_ms=120"#;
    assert_eq!(
        parse_http_line(forbidden).map(|(_, i)| i.error_status),
        Some(1)
    );
    let ok = r#"http.done url="https://api.biggo.com/x" method="GET" status=200 elapsed_ms=120"#;
    assert_eq!(parse_http_line(ok), None, "2xx 不算異常");
    let exhausted =
        r#"http.rate_limit_exhausted url="https://www.tpex.org.tw:443/a" method="GET" status=429"#;
    assert_eq!(
        parse_http_line(exhausted).map(|(h, i)| (h, i.failed)),
        Some(("www.tpex.org.tw".to_owned(), 1))
    );
    assert_eq!(
        parse_http_line("slow statement: summary=\"WITH chip\""),
        None
    );
}

/// 只讀期間內的 warn／error 檔；排名依總次數，前 5 名以外只算網域數，超過 30 次標警示。
#[test]
fn weekly_http_issues_ranks_hosts() {
    let dir =
        std::env::temp_dir().join(format!("stock_crawler_http_errors_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建立暫存目錄");
    let line = |event: &str, host: &str, status: u16| {
        format!(
            r#"2026-10-27 09:00:00.0 http.{event} url="https://{host}/p" method="GET" status={status} elapsed_ms=1"#
        )
    };
    let mut warn = Vec::new();
    for _ in 0..31 {
        warn.push(line("done", "api.biggo.com", 403));
    }
    warn.push(line("slow", "histock.tw", 200));
    warn.push(line("failed", "histock.tw", 0));
    for host in ["a.example", "b.example", "c.example", "d.example"] {
        warn.push(line("slow", host, 200));
    }
    std::fs::write(dir.join("2026-10-27_default_warn.log"), warn.join("\n")).expect("寫入");
    std::fs::write(
        dir.join("2026-10-28_default_error.log"),
        line("failed", "histock.tw", 0),
    )
    .expect("寫入");
    // 期間外與 info 檔不讀。
    std::fs::write(
        dir.join("2026-10-20_default_warn.log"),
        line("slow", "old.example", 200),
    )
    .expect("寫入");
    std::fs::write(
        dir.join("2026-10-27_default_info.log"),
        line("slow", "info.example", 200),
    )
    .expect("寫入");

    let hosts = weekly_http_issues(
        &dir,
        NaiveDate::from_ymd_opt(2026, 10, 24).expect("日期"),
        NaiveDate::from_ymd_opt(2026, 10, 30).expect("日期"),
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(hosts.len(), 6);
    assert_eq!(
        http_issue_line(&hosts),
        "本週 HTTP 異常：api.biggo.com 31 次（失敗 0／4xx-5xx 31／慢 0）、histock.tw 3 次（失敗 2／4xx-5xx 0／慢 1）、a.example 1 次（失敗 0／4xx-5xx 0／慢 1）、b.example 1 次（失敗 0／4xx-5xx 0／慢 1）、c.example 1 次（失敗 0／4xx-5xx 0／慢 1）；另 1 個網域；⚠️ api.biggo.com 一週超過 30 次，檢查來源"
    );
    assert_eq!(http_issue_line(&Default::default()), "本週 HTTP 異常 0 次");
}
