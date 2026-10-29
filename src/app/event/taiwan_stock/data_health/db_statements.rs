//! 週報的「資料庫端耗時」：`pg_stat_statements` 本週累計耗時最多的語句。
//!
//! 檔案日誌的慢查詢（[`super::slow_queries`]）只記到單筆超過門檻的語句；每次不到 1 秒、一天跑幾百次的
//! 查詢只有資料庫自己的累計值看得到。累計值從統計重置起一直加，所以每次週報把當下的累計值存進
//! Redis，下次用差額算出這一週；沒有上次的值（第一次、Redis 過期）或統計重置過時，改列自重置起的累計。

use std::collections::HashMap;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use super::*;
use crate::{domain::health::StatementStats, infra::nosql::redis::CLIENT};

/// 上次週報時的累計值。
pub(super) const SNAPSHOT_KEY: &str = "data_health:pg_stat_statements";

/// 快照保留 21 天：漏跑一週仍能以兩週的差額計算。
const SNAPSHOT_TTL_SECONDS: usize = 21 * 24 * 60 * 60;

/// 週報最多列出幾條語句。
const LISTED: usize = 3;

/// 語句內容最多顯示幾個字。
const QUERY_HEAD_CHARS: usize = 48;

/// 存進 Redis 的累計值：`queryid` → (執行次數, 累計毫秒)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct SavedStatements {
    /// 存的時候的統計重置時間；與現在不同代表中間重置過，差額不能用。
    pub(super) stats_reset: DateTime<Local>,
    /// 存的時間（週報顯示「自上次週報起」）。
    pub(super) saved_at: DateTime<Local>,
    /// 各語句的累計值。
    pub(super) totals: HashMap<i64, (i64, f64)>,
}

impl SavedStatements {
    pub(super) fn new(stats: &StatementStats, saved_at: DateTime<Local>) -> Self {
        Self {
            stats_reset: stats.stats_reset,
            saved_at,
            totals: stats
                .rows
                .iter()
                .map(|row| (row.queryid, (row.calls, row.total_ms)))
                .collect(),
        }
    }
}

/// 期間內的一條語句。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TopStatement {
    /// 語句指紋：開頭相同的語句（例如同樣以 `WITH chip AS` 起頭）靠它區分，
    /// 也可以拿去 `pg_stat_statements` 查完整內容。
    pub(super) queryid: i64,
    /// 期間內的執行次數。
    pub(super) calls: i64,
    /// 期間內的累計毫秒。
    pub(super) total_ms: f64,
    /// 語句開頭（空白合併）。
    pub(super) query: String,
}

/// 期間內累計耗時最多的語句。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TopStatements {
    /// `true` 為自上次週報起的差額，`false` 為自統計重置起的累計。
    pub(super) since_previous: bool,
    /// 期間起點。
    pub(super) since: DateTime<Local>,
    /// 依期間內累計毫秒由多到少。
    pub(super) top: Vec<TopStatement>,
}

/// 以 `previous` 為起點算期間內的前 `limit` 名；`previous` 不能用時以統計重置為起點。
///
/// 被擴充淘汰後重新計數的語句，累計值會比上次小，差額不是正數的不列。
pub(super) fn top_statements(
    previous: Option<&SavedStatements>,
    now: &StatementStats,
    limit: usize,
) -> TopStatements {
    let base = previous.filter(|saved| saved.stats_reset == now.stats_reset);
    let mut top: Vec<TopStatement> = now
        .rows
        .iter()
        .filter_map(|row| {
            let (calls, total_ms) = base
                .and_then(|saved| saved.totals.get(&row.queryid))
                .copied()
                .unwrap_or((0, 0.0));
            let calls = row.calls - calls;
            let total_ms = row.total_ms - total_ms;
            (calls > 0 && total_ms > 0.0).then(|| TopStatement {
                queryid: row.queryid,
                calls,
                total_ms,
                query: query_head(&row.query),
            })
        })
        .collect();
    top.sort_by(|a, b| b.total_ms.total_cmp(&a.total_ms));
    top.truncate(limit);
    TopStatements {
        since_previous: base.is_some(),
        since: base.map_or(now.stats_reset, |saved| saved.saved_at),
        top,
    }
}

/// 空白合併成一格，超過 [`QUERY_HEAD_CHARS`] 個字就截斷加「…」。
fn query_head(query: &str) -> String {
    let flat = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= QUERY_HEAD_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(QUERY_HEAD_CHARS).collect();
    format!("{head}…")
}

/// 週報最後附的一行（未跳脫）。`stats` 為 `Err` 代表讀取失敗（已記在日誌）。
pub(super) fn statement_line(
    previous: Option<&SavedStatements>,
    stats: &std::result::Result<Option<StatementStats>, ()>,
) -> String {
    let stats = match stats {
        Ok(Some(stats)) => stats,
        Ok(None) => return "資料庫端耗時：沒有安裝 pg_stat_statements".to_string(),
        Err(()) => return "資料庫端耗時：讀取 pg_stat_statements 失敗（見日誌）".to_string(),
    };
    let top = top_statements(previous, stats, LISTED);
    let since = format!(
        "{}{}起",
        if top.since_previous {
            "上次週報 "
        } else {
            "統計重置 "
        },
        top.since.format("%m-%d %H:%M")
    );
    if top.top.is_empty() {
        return format!("資料庫端耗時：{since}沒有新的語句");
    }
    let items = top
        .top
        .iter()
        .enumerate()
        .map(|(index, item)| {
            format!(
                "{}. {}（#{}）{} 次、平均 {:.0} ms、共 {:.0} 秒",
                index + 1,
                item.query,
                item.queryid,
                item.calls,
                item.total_ms / item.calls as f64,
                item.total_ms / 1000.0,
            )
        })
        .collect::<Vec<_>>()
        .join("；");
    format!("資料庫端耗時前 {}（{since}）：{items}", top.top.len())
}

/// 讀資料庫端累計值與上次週報的快照，回傳週報那一行與這次的累計值（發送後再存）。
pub(super) async fn weekly_statements(
    repo: &impl DataHealthRepository,
) -> (String, Option<StatementStats>) {
    let stats = repo.fetch_statement_stats().await.map_err(|why| {
        tracing::warn!("讀取 pg_stat_statements 失敗，週報略過資料庫端耗時: {why:?}");
    });
    let previous = match &stats {
        Ok(Some(_)) => load(SNAPSHOT_KEY).await,
        _ => None,
    };
    let line = statement_line(previous.as_ref(), &stats);
    (line, stats.ok().flatten())
}

/// 上次週報存在 `key` 的快照；沒有、過期或格式不符時回 `None`。
pub(super) async fn load(key: &str) -> Option<SavedStatements> {
    let text = CLIENT.get_string(key).await.ok()?;
    serde_json::from_str(&text).ok()
}

/// 把這次的累計值存到 `key`，給下次週報算差額；失敗只記日誌。
pub(super) async fn save(key: &str, stats: &StatementStats) {
    let saved = SavedStatements::new(stats, Local::now());
    let result = match serde_json::to_string(&saved) {
        Ok(text) => CLIENT
            .set(key, text, SNAPSHOT_TTL_SECONDS)
            .await
            .map_err(anyhow::Error::from),
        Err(why) => Err(why.into()),
    };
    if let Err(why) = result {
        tracing::warn!("存 pg_stat_statements 快照失敗，下次週報改列累計值: {why:?}");
    }
}
