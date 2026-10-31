//! 週報的「本週異常價格」：掃 `log/` 底下期間內的 info 檔（含輪轉出來的舊檔），彙總每天收盤時
//! [`flush_abnormal_price_stats`](crate::infra::cache::flush_abnormal_price_stats) 寫下的
//! 「今日過濾異常價格：…」，依站點加總被擋次數與發生天數。
//!
//! 被擋的價格不會進入快取（驗價已擋下），但某站一週錯很多次代表來源不穩，值得換掉或降低權重；
//! 單站一週超過 [`SITE_WARN_COUNT`] 次就在週報標出來。只讀本機檔案，不碰資料庫。

use std::collections::BTreeMap;
use std::path::Path;

use super::*;

/// 收盤統計行的開頭（與 `infra::cache::abnormal_price` 的輸出一致）。
const DAILY_PREFIX: &str = "今日過濾異常價格：";

/// 單站一週被擋超過這個次數就標警示。
const SITE_WARN_COUNT: usize = 50;

/// 一個站點本週被擋的次數與發生天數。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct SiteWeek {
    count: usize,
    days: usize,
}

/// 期間內（含起訖）每天 info 檔裡的異常價格統計，依站點加總；目錄或檔案讀不到時略過。
pub(super) fn weekly_abnormal_prices(
    dir: &Path,
    from: NaiveDate,
    to: NaiveDate,
) -> BTreeMap<String, SiteWeek> {
    let mut daily: BTreeMap<(String, NaiveDate), usize> = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return BTreeMap::new();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.contains("_default_info") {
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
        for (site, count) in text.lines().flat_map(parse_daily_line) {
            *daily.entry((site, date)).or_default() += count;
        }
    }
    let mut weekly: BTreeMap<String, SiteWeek> = BTreeMap::new();
    for ((site, _), count) in daily {
        let week = weekly.entry(site).or_default();
        week.count += count;
        week.days += 1;
    }
    weekly
}

/// 解析一行收盤統計，例如 `今日過濾異常價格：HiStock 3 次（2 檔：3189、3481）；CnYes 1 次（1 檔：2330）`；
/// 不是統計行時為空。
fn parse_daily_line(line: &str) -> Vec<(String, usize)> {
    let Some((_, sites)) = line.split_once(DAILY_PREFIX) else {
        return Vec::new();
    };
    sites
        .split('；')
        .filter_map(|part| {
            let (head, _) = part.split_once(" 次（")?;
            let (site, count) = head.rsplit_once(' ')?;
            Some((site.trim().to_owned(), count.parse().ok()?))
        })
        .collect()
}

/// 週報附的一行：各站本週被擋次數與天數，超過門檻的站點另外標示。
pub(super) fn abnormal_price_line(weekly: &BTreeMap<String, SiteWeek>) -> String {
    if weekly.is_empty() {
        return "本週異常價格 0 次".to_string();
    }
    let sites: Vec<String> = weekly
        .iter()
        .map(|(site, week)| format!("{site} {} 次（{} 天）", week.count, week.days))
        .collect();
    let noisy: Vec<&str> = weekly
        .iter()
        .filter(|(_, week)| week.count > SITE_WARN_COUNT)
        .map(|(site, _)| site.as_str())
        .collect();
    let warning = if noisy.is_empty() {
        String::new()
    } else {
        format!(
            "；⚠️ {} 一週超過 {SITE_WARN_COUNT} 次，檢查來源",
            noisy.join("、")
        )
    };
    format!("本週異常價格：{}{warning}", sites.join("、"))
}
