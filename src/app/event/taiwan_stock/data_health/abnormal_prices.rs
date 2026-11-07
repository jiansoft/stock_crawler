//! 週報的「本週異常價格」：掃 `log/` 底下期間內的 info 檔（含輪轉出來的舊檔），彙總每天收盤時
//! [`flush_abnormal_price_stats`](crate::infra::cache::flush_abnormal_price_stats) 寫下的
//! 「今日過濾異常價格：…」，依站點加總被擋次數與發生天數，並列出被擋天數最多的股票與最近一次的
//! 採集價（11-07 起收盤統計每檔附 `採集價／昨收 昨收價`，之前的舊格式只有代號）。
//!
//! 被擋的價格不會進入快取（驗價已擋下），但某站一週錯很多次代表來源不穩，值得換掉或降低權重；
//! 單站一週超過 [`SITE_WARN_COUNT`] 次就在週報標出來。只讀本機檔案，不碰資料庫。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::*;

/// 收盤統計行的開頭（與 `infra::cache::abnormal_price` 的輸出一致）。
const DAILY_PREFIX: &str = "今日過濾異常價格：";

/// 單站一週被擋超過這個次數就標警示。
const SITE_WARN_COUNT: usize = 50;

/// 週報列出被擋天數最多的股票數。
const TOP_SYMBOLS: usize = 3;

/// 一個站點本週被擋的次數與發生天數。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct SiteWeek {
    count: usize,
    days: usize,
}

/// 一檔股票本週被擋的日子與最近一天的採集價說明（例如 `452.96／昨收 900`；舊格式沒有）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct SymbolWeek {
    days: BTreeSet<NaiveDate>,
    latest: Option<(NaiveDate, String)>,
}

/// 本週的異常價格：依站點的次數與天數，與依股票的被擋天數。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct AbnormalWeek {
    sites: BTreeMap<String, SiteWeek>,
    symbols: BTreeMap<String, SymbolWeek>,
}

/// 收盤統計行裡一個站點的內容。
#[derive(Debug, PartialEq, Eq)]
struct DailySite {
    site: String,
    count: usize,
    /// `(代號, 採集價說明)`；舊格式與省略號帶過的代號沒有說明。
    symbols: Vec<(String, Option<String>)>,
}

/// 期間內（含起訖）每天 info 檔裡的異常價格統計，依站點與股票加總；目錄或檔案讀不到時略過。
pub(super) fn weekly_abnormal_prices(dir: &Path, from: NaiveDate, to: NaiveDate) -> AbnormalWeek {
    let mut daily: BTreeMap<(String, NaiveDate), usize> = BTreeMap::new();
    let mut symbols: BTreeMap<String, SymbolWeek> = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return AbnormalWeek::default();
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
        for line in text.lines().flat_map(parse_daily_line) {
            *daily.entry((line.site, date)).or_default() += line.count;
            for (symbol, detail) in line.symbols {
                let week = symbols.entry(symbol).or_default();
                week.days.insert(date);
                let newer = week.latest.as_ref().is_none_or(|(day, _)| *day <= date);
                if let Some(detail) = detail.filter(|_| newer) {
                    week.latest = Some((date, detail));
                }
            }
        }
    }
    let mut weekly: BTreeMap<String, SiteWeek> = BTreeMap::new();
    for ((site, _), count) in daily {
        let week = weekly.entry(site).or_default();
        week.count += count;
        week.days += 1;
    }
    AbnormalWeek {
        sites: weekly,
        symbols,
    }
}

/// 解析一行收盤統計，例如 `今日過濾異常價格：HiStock 3 次（2 檔：3189 452.96／昨收 900、3481 1.59／昨收 42.5）；
/// CnYes 1 次（1 檔：2330）`（舊格式只有代號）；不是統計行時為空。
fn parse_daily_line(line: &str) -> Vec<DailySite> {
    let Some((_, sites)) = line.split_once(DAILY_PREFIX) else {
        return Vec::new();
    };
    sites.split('；').filter_map(parse_daily_site).collect()
}

/// 解析收盤統計裡的一個站點：`HiStock 3 次（2 檔：3189 452.96／昨收 900、3481…）`。
fn parse_daily_site(part: &str) -> Option<DailySite> {
    let (head, rest) = part.split_once(" 次（")?;
    let (site, count) = head.rsplit_once(' ')?;
    let listed = rest
        .split_once("檔：")
        .map_or("", |(_, listed)| listed.trim_end().trim_end_matches('）'));
    let symbols = listed
        .split('、')
        .filter_map(|item| {
            let item = item.trim().trim_end_matches('…');
            let (symbol, detail) = match item.split_once(' ') {
                Some((symbol, detail)) => (symbol, Some(detail.trim().to_owned())),
                None => (item, None),
            };
            (!symbol.is_empty()).then(|| (symbol.to_owned(), detail))
        })
        .collect();
    Some(DailySite {
        site: site.trim().to_owned(),
        count: count.parse().ok()?,
        symbols,
    })
}

/// 週報附的一行：各站本週被擋次數與天數，超過門檻的站點另外標示，最後列被擋天數最多的股票。
pub(super) fn abnormal_price_line(abnormal: &AbnormalWeek) -> String {
    let weekly = &abnormal.sites;
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
    format!(
        "本週異常價格：{}{warning}{}",
        sites.join("、"),
        top_symbols(&abnormal.symbols)
    )
}

/// 被擋天數最多的幾檔（同天數依代號），附最近一次的採集價；沒有代號時為空字串。
fn top_symbols(symbols: &BTreeMap<String, SymbolWeek>) -> String {
    let mut ranked: Vec<(&String, &SymbolWeek)> = symbols.iter().collect();
    ranked.sort_by(|a, b| b.1.days.len().cmp(&a.1.days.len()).then(a.0.cmp(b.0)));
    let listed: Vec<String> = ranked
        .into_iter()
        .take(TOP_SYMBOLS)
        .map(|(symbol, week)| match &week.latest {
            Some((_, detail)) => format!("{symbol} {} 天（{detail}）", week.days.len()),
            None => format!("{symbol} {} 天", week.days.len()),
        })
        .collect();
    if listed.is_empty() {
        String::new()
    } else {
        format!("；最常被擋：{}", listed.join("、"))
    }
}
