//! 週報的「本週 HTTP 異常」：掃 `log/` 底下期間內的 warn／error 檔（含輪轉出來的舊檔），依網域加總
//! `http.failed`（含快速失敗）、`http.done`（4xx／5xx）、`http.slow` 與 429 用盡的次數。
//!
//! 站點出問題時（例如 10-27 BigGo 一天 44 次 403、PcHome 92 次逾時）每筆 WARN 散落在日誌裡，
//! 週報把最常出錯的網域列出來，單一網域一週超過 [`HOST_WARN_COUNT`] 次就標警示。只讀本機檔案。

use std::collections::BTreeMap;
use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use super::*;

/// 週報最多列出幾個網域。
const LISTED_HOSTS: usize = 5;

/// 單一網域一週異常超過這個次數就標警示。
const HOST_WARN_COUNT: usize = 30;

/// `http.<事件>` 與網址的網域。
static EVENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bhttp\.([a-z_.]+)").expect("http 事件正規式應合法"));
static HOST_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"url="https?://([^/":?]+)"#).expect("網域正規式應合法"));
static STATUS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bstatus=(\d{3})").expect("狀態碼正規式應合法"));

/// 一個網域本週的異常次數。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct HostIssues {
    /// 連線或請求失敗（含快速失敗、429 重試用盡）。
    pub(super) failed: usize,
    /// 回應 4xx／5xx。
    pub(super) error_status: usize,
    /// 回應超過慢請求門檻。
    pub(super) slow: usize,
}

impl HostIssues {
    fn total(&self) -> usize {
        self.failed + self.error_status + self.slow
    }
}

/// 一行日誌的網域與異常種類；不是 HTTP 異常時為 `None`。
pub(super) fn parse_http_line(line: &str) -> Option<(String, HostIssues)> {
    let event = EVENT_RE.captures(line)?.get(1)?.as_str();
    let host = HOST_RE.captures(line)?.get(1)?.as_str().to_owned();
    let mut issues = HostIssues::default();
    match event {
        "slow" => issues.slow = 1,
        "done" => {
            let status: u16 = STATUS_RE.captures(line)?.get(1)?.as_str().parse().ok()?;
            if status < 400 {
                return None;
            }
            issues.error_status = 1;
        }
        e if e.starts_with("failed") || e == "rate_limit_exhausted" => issues.failed = 1,
        _ => return None,
    }
    Some((host, issues))
}

/// 期間內（含起訖）warn／error 檔裡的 HTTP 異常，依網域加總；目錄或檔案讀不到時略過。
pub(super) fn weekly_http_issues(
    dir: &Path,
    from: NaiveDate,
    to: NaiveDate,
) -> BTreeMap<String, HostIssues> {
    let mut hosts: BTreeMap<String, HostIssues> = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return hosts;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.contains("_default_warn") && !name.contains("_default_error") {
            continue;
        }
        let Some(date) = name
            .get(..10)
            .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
        else {
            continue;
        };
        if date < from || date > to {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for (host, issues) in text.lines().filter_map(parse_http_line) {
            let total = hosts.entry(host).or_default();
            total.failed += issues.failed;
            total.error_status += issues.error_status;
            total.slow += issues.slow;
        }
    }
    hosts
}

/// 週報附的一行：異常最多的前 [`LISTED_HOSTS`] 個網域（失敗／4xx-5xx／慢），超過門檻的另外標示。
pub(super) fn http_issue_line(hosts: &BTreeMap<String, HostIssues>) -> String {
    if hosts.is_empty() {
        return "本週 HTTP 異常 0 次".to_string();
    }
    let mut ranked: Vec<(&String, &HostIssues)> = hosts.iter().collect();
    ranked.sort_by(|(a_host, a), (b_host, b)| b.total().cmp(&a.total()).then(a_host.cmp(b_host)));
    let listed: Vec<String> = ranked
        .iter()
        .take(LISTED_HOSTS)
        .map(|(host, issues)| {
            format!(
                "{host} {} 次（失敗 {}／4xx-5xx {}／慢 {}）",
                issues.total(),
                issues.failed,
                issues.error_status,
                issues.slow
            )
        })
        .collect();
    let more = ranked.len().saturating_sub(LISTED_HOSTS);
    let more = if more > 0 {
        format!("；另 {more} 個網域")
    } else {
        String::new()
    };
    let noisy: Vec<&str> = ranked
        .iter()
        .filter(|(_, issues)| issues.total() > HOST_WARN_COUNT)
        .map(|(host, _)| host.as_str())
        .collect();
    let warning = if noisy.is_empty() {
        String::new()
    } else {
        format!(
            "；⚠️ {} 一週超過 {HOST_WARN_COUNT} 次，檢查來源",
            noisy.join("、")
        )
    };
    format!("本週 HTTP 異常：{}{more}{warning}", listed.join("、"))
}
