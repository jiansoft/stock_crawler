//! 週報的「本週取連線過慢」：掃 `log/` 底下期間內的 warn 檔（含輪轉出來的舊檔），統計 sqlx 的
//! `acquired connection, but time to acquire exceeded slow threshold` 次數、最長等待與最集中的一分鐘。
//!
//! 連線池被長查詢占滿或資料庫主機忙時，取連線會先變慢、之後才逾時（10-29 15:00 收盤一秒內 8 筆）；
//! 同一分鐘集中出現代表是同一批工作在搶連線。只讀本機檔案。

use std::collections::BTreeMap;
use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use super::*;

/// sqlx 取連線過慢的訊息。
const SLOW_ACQUIRE_MESSAGE: &str = "time to acquire exceeded slow threshold";

/// 日誌開頭的「YYYY-MM-DD HH:MM」與等待秒數。
static MINUTE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2})").expect("時間正規式應合法"));
static SECS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bacquired_after_secs=([0-9.]+)").expect("等待秒數正規式應合法"));

/// 本週取連線過慢的統計。
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct SlowAcquires {
    /// 每分鐘（`YYYY-MM-DD HH:MM`）的次數。
    pub(super) by_minute: BTreeMap<String, usize>,
    /// 最長等待秒數。
    pub(super) max_secs: f64,
}

impl SlowAcquires {
    fn total(&self) -> usize {
        self.by_minute.values().sum()
    }
}

/// 一行日誌的分鐘與等待秒數；不是取連線過慢時為 `None`（沒有秒數時記 0）。
pub(super) fn parse_acquire_line(line: &str) -> Option<(String, f64)> {
    if !line.contains(SLOW_ACQUIRE_MESSAGE) {
        return None;
    }
    let minute = MINUTE_RE.captures(line)?.get(1)?.as_str().to_owned();
    let secs = SECS_RE
        .captures(line)
        .and_then(|caps| caps.get(1)?.as_str().parse().ok())
        .unwrap_or(0.0);
    Some((minute, secs))
}

/// 期間內（含起訖）warn 檔裡的取連線過慢；目錄或檔案讀不到時略過。
pub(super) fn weekly_slow_acquires(dir: &Path, from: NaiveDate, to: NaiveDate) -> SlowAcquires {
    let mut acquires = SlowAcquires::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return acquires;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.contains("_default_warn") {
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
        for (minute, secs) in text.lines().filter_map(parse_acquire_line) {
            *acquires.by_minute.entry(minute).or_default() += 1;
            acquires.max_secs = acquires.max_secs.max(secs);
        }
    }
    acquires
}

/// 週報附的一行：次數、最長等待，以及次數最多的那一分鐘（同次數取最早）。
pub(super) fn slow_acquire_line(acquires: &SlowAcquires) -> String {
    let total = acquires.total();
    if total == 0 {
        return "本週取連線過慢 0 次".to_string();
    }
    // 依時間順序走訪，只在次數更多時替換，同次數保留較早的一分鐘。
    let mut peak: Option<(&String, usize)> = None;
    for (minute, &count) in &acquires.by_minute {
        if peak.is_none_or(|(_, best)| count > best) {
            peak = Some((minute, count));
        }
    }
    let peak = match peak {
        Some((minute, count)) if count > 1 => {
            // 「YYYY-MM-DD HH:MM」只留月日與時分。
            format!(
                "，最集中在 {} 的 {count} 次",
                minute.get(5..).unwrap_or(minute)
            )
        }
        _ => String::new(),
    };
    format!(
        "本週取連線過慢 {total} 次（最長 {:.1} 秒{peak}）",
        acquires.max_secs
    )
}
