//! 慢查詢的當日統計：[`super::FileLogLayer`] 收到 sqlx 的 slow statement 事件時依 SQL 開頭歸類記一筆，
//! 主機健康每日彙總（23:30）以 [`daily_summary`] 附在彙總行後面。
//!
//! 統計只在記憶體裡、跨日歸零；重啟會清掉，所以行程在當天啟動時文字會註明「自 HH:MM 起」。

use std::{collections::HashMap, sync::Mutex};

use chrono::{Local, NaiveDate, NaiveDateTime};
use once_cell::sync::Lazy;

/// 每日彙總最多列出幾類，其餘只計入總筆數。
const LISTED_KINDS: usize = 3;

/// sqlx 慢查詢事件的 target。
const SQLX_TARGET: &str = "sqlx::query";

/// 只有慢查詢事件才帶的欄位（一般 statement 事件沒有）。
const SLOW_THRESHOLD_FIELD: &str = "slow_threshold";

/// 行程內唯一的統計；起點是 [`init`] 被呼叫的時間（程式啟動時）。
static STATS: Lazy<Mutex<SlowQueryStats>> =
    Lazy::new(|| Mutex::new(SlowQueryStats::new(Local::now().naive_local())));

/// 單一類別的筆數與最長耗時。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct KindStats {
    count: u32,
    max_secs: f64,
}

/// 當日各類慢查詢的筆數與最長耗時。
#[derive(Debug)]
pub(crate) struct SlowQueryStats {
    /// 行程開始統計的時間。
    started: NaiveDateTime,
    /// 目前累計的日期；跨日第一筆時歸零。
    day: Option<NaiveDate>,
    /// 依 sqlx `summary`（SQL 前 4 個字）歸類。
    kinds: HashMap<String, KindStats>,
}

impl SlowQueryStats {
    pub(crate) fn new(started: NaiveDateTime) -> Self {
        Self {
            started,
            day: None,
            kinds: HashMap::new(),
        }
    }

    /// 記一筆慢查詢；`summary` 是 sqlx 的摘要（截斷時結尾帶「 …」）。
    pub(crate) fn record(&mut self, now: NaiveDateTime, summary: &str, secs: f64) {
        let day = now.date();
        if self.day != Some(day) {
            self.day = Some(day);
            self.kinds.clear();
        }
        let kind = summary.trim_end_matches('…').trim_end();
        let stats = self.kinds.entry(kind.to_string()).or_default();
        stats.count += 1;
        stats.max_secs = stats.max_secs.max(secs);
    }

    /// `today` 的彙總文字：總筆數、統計起點（當天啟動時），以及筆數最多的前幾類與各自最長耗時。
    pub(crate) fn summary(&self, today: NaiveDate) -> String {
        let since = if self.started.date() == today {
            format!("（自 {} 起）", self.started.format("%H:%M"))
        } else {
            String::new()
        };
        let mut kinds: Vec<(&String, &KindStats)> = if self.day == Some(today) {
            self.kinds.iter().collect()
        } else {
            Vec::new()
        };
        let total: u32 = kinds.iter().map(|(_, stats)| stats.count).sum();
        if total == 0 {
            return format!("慢查詢 0 筆{since}");
        }
        kinds.sort_by(|(a_kind, a), (b_kind, b)| {
            b.count
                .cmp(&a.count)
                .then(b.max_secs.total_cmp(&a.max_secs))
                .then(a_kind.cmp(b_kind))
        });
        let listed: Vec<String> = kinds
            .iter()
            .take(LISTED_KINDS)
            .map(|(kind, stats)| format!("{kind} {}（最長 {:.1} 秒）", stats.count, stats.max_secs))
            .collect();
        let others = if kinds.len() > LISTED_KINDS {
            format!(" 等 {} 類", kinds.len())
        } else {
            String::new()
        };
        format!("慢查詢 {total} 筆{since}：{}{others}", listed.join("、"))
    }
}

/// 開始統計（程式啟動、subscriber 初始化後呼叫）：把起點定在啟動時間，而不是第一筆慢查詢的時間。
pub fn init() {
    Lazy::force(&STATS);
}

/// 若事件是 sqlx 的慢查詢就記一筆；由 [`super::FileLogLayer`] 對每個事件呼叫。
pub(super) fn observe(target: &str, level: tracing::Level, extra: &[(String, serde_json::Value)]) {
    if target != SQLX_TARGET || level != tracing::Level::WARN {
        return;
    }
    let Some((summary, secs)) = slow_statement_fields(extra) else {
        return;
    };
    if let Ok(mut stats) = STATS.lock() {
        stats.record(Local::now().naive_local(), summary, secs);
    }
}

/// 從事件欄位取出 `summary` 與 `elapsed_secs`；沒有 `slow_threshold`（不是慢查詢）回 `None`。
fn slow_statement_fields(extra: &[(String, serde_json::Value)]) -> Option<(&str, f64)> {
    let field = |name: &str| extra.iter().find(|(key, _)| key == name).map(|(_, v)| v);
    field(SLOW_THRESHOLD_FIELD)?;
    let summary = field("summary")?.as_str()?;
    let secs = field("elapsed_secs")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default();
    Some((summary, secs))
}

/// 今天的慢查詢彙總文字（見 [`SlowQueryStats::summary`]）。
pub fn daily_summary(today: NaiveDate) -> String {
    STATS
        .lock()
        .map(|stats| stats.summary(today))
        .unwrap_or_else(|_| "慢查詢 —".to_string())
}

#[cfg(test)]
mod tests;
