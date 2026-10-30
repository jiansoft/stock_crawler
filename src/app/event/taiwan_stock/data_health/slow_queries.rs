//! 週報的「本週慢查詢」：掃 `log/` 底下期間內的 warn 檔（含輪轉出來的舊檔），依 SQL 開頭歸類。
//!
//! 23:30 主機健康彙總的慢查詢統計（[`logging::slow_query`]）只在記憶體，重啟就清掉；週報改讀檔案，
//! 跨重啟也算得到整週。只讀本機檔案，不碰資料庫。

use std::path::Path;

use super::*;
use crate::core::logging::{self, slow_query::SlowQueryTally};

/// 檔案日誌的目錄（與主機健康的探針同一個相對路徑）。
pub(super) const LOG_DIR: &str = "log";

/// 週報最多列出幾類。
const LISTED_KINDS: usize = 5;

/// 期間內（含起訖）每天的 warn 檔裡的慢查詢；目錄或檔案讀不到時略過。
///
/// 檔名格式為 `YYYY-MM-DD_default_warn.log` 或輪轉後的 `YYYY-MM-DD_default_warn.HH-MM-SS.log`。
pub(super) fn weekly_slow_queries(dir: &Path, from: NaiveDate, to: NaiveDate) -> SlowQueryTally {
    let mut tally = SlowQueryTally::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return tally;
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
        for (summary, secs, origin) in text.lines().filter_map(logging::slow_query::parse_log_line)
        {
            tally.add(&summary, secs, origin);
        }
    }
    tally
}

/// 週報最後附的一行：總筆數、依來源（排程／API／其他）的筆數與前 [`LISTED_KINDS`] 類。
pub(super) fn slow_query_line(tally: &SlowQueryTally) -> String {
    match tally.top_kinds(LISTED_KINDS) {
        Some(kinds) => format!(
            "本週慢查詢 {} 筆{}：{kinds}",
            tally.total(),
            tally.origins()
        ),
        None => "本週慢查詢 0 筆".to_string(),
    }
}
