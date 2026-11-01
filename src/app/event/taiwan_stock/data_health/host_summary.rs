//! 週報的「本週主機健康」：掃 `log/` 底下期間內的 info 檔（含輪轉出來的舊檔），彙總每天 23:30
//! 主機健康彙總那一行（fsync、MemAvailable、swap、供電／降頻、資料庫連不上次數）。
//!
//! 每日彙總只在 Telegram 以外的日誌裡，一週下來不會有人逐日翻；週報把最差的一天標出來。
//! 行程當天重啟時彙總只涵蓋重啟之後，同一天有多行時各欄位取最差值。只讀本機檔案。

use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use super::*;

/// 每日彙總行的開頭與日期。
static DAY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"主機健康彙總 (\d{4}-\d{2}-\d{2})：").expect("主機健康彙總日期的正規式應合法")
});
static FSYNC_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"fsync 最大 (\d+) ms(?:、逾時或失敗 (\d+) 次)?").expect("fsync 正規式應合法")
});
static AVAILABLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"MemAvailable 最低 (\d+) MB").expect("MemAvailable 正規式應合法"));
static SWAP_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"swap 最高 ([\d.]+)%").expect("swap 正規式應合法"));
static THROTTLED_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"供電／降頻 目前 ([^；｜]+)").expect("供電正規式應合法"));
static DB_DOWN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"資料庫：連不上 (\d+) 次").expect("資料庫正規式應合法"));

/// 一天的主機健康彙總；欄位缺漏（例如當天略過 fsync 探針）為 `None`。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct HostDay {
    pub(super) date: Option<NaiveDate>,
    pub(super) max_fsync_ms: Option<u64>,
    pub(super) stuck_probes: u32,
    pub(super) min_available_mb: Option<u64>,
    pub(super) max_swap_percent: Option<f64>,
    pub(super) throttled: bool,
    pub(super) db_unreachable: u32,
}

/// 解析一行每日彙總；不是彙總行時為 `None`。
pub(super) fn parse_host_line(line: &str) -> Option<HostDay> {
    let date = DAY_RE
        .captures(line)
        .and_then(|caps| NaiveDate::parse_from_str(&caps[1], "%Y-%m-%d").ok())?;
    let number = |re: &Regex, group: usize| -> Option<String> {
        re.captures(line)
            .and_then(|caps| caps.get(group).map(|m| m.as_str().to_owned()))
    };
    Some(HostDay {
        date: Some(date),
        max_fsync_ms: number(&FSYNC_RE, 1).and_then(|v| v.parse().ok()),
        stuck_probes: number(&FSYNC_RE, 2)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        min_available_mb: number(&AVAILABLE_RE, 1).and_then(|v| v.parse().ok()),
        max_swap_percent: number(&SWAP_RE, 1).and_then(|v| v.parse().ok()),
        throttled: number(&THROTTLED_RE, 1).is_some_and(|v| v.trim() != "無"),
        db_unreachable: number(&DB_DOWN_RE, 1)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    })
}

/// 期間內（含起訖）每天 info 檔裡的主機健康彙總，依日期由舊到新；同一天多行時各欄位取最差值。
pub(super) fn weekly_host_days(dir: &Path, from: NaiveDate, to: NaiveDate) -> Vec<HostDay> {
    let mut days: Vec<HostDay> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return days;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.contains("_default_info") {
            continue;
        }
        // 檔名日期在期間外的不讀：23:30 的彙總寫在當天的檔案裡。
        let Some(file_date) = name
            .get(..10)
            .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
        else {
            continue;
        };
        if file_date < from || file_date > to {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for day in text.lines().filter_map(parse_host_line) {
            let Some(date) = day.date.filter(|date| *date >= from && *date <= to) else {
                continue;
            };
            match days.iter_mut().find(|known| known.date == Some(date)) {
                Some(known) => merge_worst(known, &day),
                None => days.push(day),
            }
        }
    }
    days.sort_by_key(|day| day.date);
    days
}

/// 同一天的兩行合併成最差值。
fn merge_worst(known: &mut HostDay, other: &HostDay) {
    known.max_fsync_ms = known.max_fsync_ms.max(other.max_fsync_ms);
    known.stuck_probes += other.stuck_probes;
    known.min_available_mb = match (known.min_available_mb, other.min_available_mb) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    known.max_swap_percent = match (known.max_swap_percent, other.max_swap_percent) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    known.throttled |= other.throttled;
    known.db_unreachable += other.db_unreachable;
}

/// 週報附的一行：fsync 最大（與日期）與逾時次數、MemAvailable 最低、swap 最高、供電／降頻天數、
/// 資料庫連不上次數。
pub(super) fn host_summary_line(days: &[HostDay]) -> String {
    if days.is_empty() {
        return "本週主機健康：沒有每日彙總".to_string();
    }
    let date_of = |day: &HostDay| {
        day.date
            .map_or_else(String::new, |date| date.format("%m-%d").to_string())
    };
    let fsync = days
        .iter()
        .filter_map(|day| day.max_fsync_ms.map(|ms| (ms, day)))
        .max_by_key(|(ms, _)| *ms)
        .map_or_else(
            || "fsync —".to_owned(),
            |(ms, day)| format!("fsync 最大 {ms} ms（{}）", date_of(day)),
        );
    let stuck: u32 = days.iter().map(|day| day.stuck_probes).sum();
    let available = days
        .iter()
        .filter_map(|day| day.min_available_mb.map(|mb| (mb, day)))
        .min_by_key(|(mb, _)| *mb)
        .map_or_else(
            || "MemAvailable —".to_owned(),
            |(mb, day)| format!("MemAvailable 最低 {mb} MB（{}）", date_of(day)),
        );
    let swap = days
        .iter()
        .filter_map(|day| day.max_swap_percent.map(|percent| (percent, day)))
        .max_by(|(a, _), (b, _)| a.total_cmp(b))
        .map_or_else(
            || "swap —".to_owned(),
            |(percent, day)| format!("swap 最高 {percent:.1}%（{}）", date_of(day)),
        );
    let throttled = days.iter().filter(|day| day.throttled).count();
    let db_unreachable: u32 = days.iter().map(|day| day.db_unreachable).sum();
    format!(
        "本週主機健康（{} 天）：{fsync}、逾時 {stuck} 次｜{available}｜{swap}｜供電／降頻 {throttled} 天｜資料庫連不上 {db_unreachable} 次",
        days.len()
    )
}
