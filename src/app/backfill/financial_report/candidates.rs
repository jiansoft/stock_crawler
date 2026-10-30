//! 候選股票排序（季報期優先）與逐檔採集迴圈。

use std::{collections::HashSet, future::Future};

use anyhow::{Context, Result};

use super::{
    PRIORITY_LOOKBACK_DAYS, PRIORITY_TTL_SECONDS, REFRESH_TTL_SECONDS, RunSummary,
    fetch::StockReports, is_candidate, make_cache_key, make_priority_cache_key, save_biggo_reports,
    save_reports,
};
use crate::{
    domain::{financial::repository::FinancialReportRepository, registry::entity::Stock},
    infra::{crawler::yahoo, nosql::redis::CLIENT},
};

/// 從股票主檔挑出候選股票並排出採集順序：損益表落後官方季報的在前，其餘依代號。
///
/// 回傳順序與優先檔數；查不到落後名單只是少了優先順序，照一般輪替採集即可。
pub(super) async fn order_candidates(
    repo: &impl FinancialReportRepository,
    stocks: &[Stock],
) -> (Vec<String>, usize) {
    let mut candidates: Vec<String> = stocks
        .iter()
        .filter(|stock| is_candidate(stock))
        .map(|stock| stock.symbol().0.clone())
        .collect();
    candidates.sort();

    let lagging: Vec<String> = repo
        .fetch_lagging_statements(PRIORITY_LOOKBACK_DAYS, 0)
        .await
        .map(|statements| {
            statements
                .into_iter()
                .map(|statement| statement.stock_symbol)
                .collect()
        })
        .unwrap_or_else(|why| {
            tracing::warn!("fetch lagging financial report symbols failed: {why:#}");
            Vec::new()
        });
    let (candidates, priority) = prioritize(candidates, &lagging);
    tracing::info!(
        "Yahoo 財報採集開始: candidates={}, priority={priority}",
        candidates.len()
    );
    (candidates, priority)
}

/// 把損益表落後官方季報的候選股票移到最前面（各自維持原順序），回傳新順序與優先檔數。
///
/// 不在候選名單裡的落後代號（如興櫃、ETF）不採集。
pub(super) fn prioritize(candidates: Vec<String>, lagging: &[String]) -> (Vec<String>, usize) {
    let lagging: HashSet<&str> = lagging.iter().map(String::as_str).collect();
    let (mut ordered, rest): (Vec<String>, Vec<String>) = candidates
        .into_iter()
        .partition(|symbol| lagging.contains(symbol.as_str()));
    let priority = ordered.len();
    ordered.extend(rest);
    (ordered, priority)
}

/// 依序採集候選股票，跳過 Redis 旗標內處理過的，至多實際嘗試 `max_stocks` 檔。
///
/// 前 `priority` 檔是優先股票：只看 [`PRIORITY_TTL_SECONDS`] 的優先旗標、不看每週旗標，
/// 嘗試時兩個旗標都寫。Yahoo 失敗（404 除外）時改從 BigGo 採集。`yahoo`、`biggo` 是
/// 單檔報表的抓取函式，`pause` 是股票之間的間隔；測試時換成假資料與不等待。
pub(super) async fn run_candidates<'a, Y, YF, B, BF, P, PF>(
    candidates: &'a [String],
    priority: usize,
    max_stocks: usize,
    repo: &impl FinancialReportRepository,
    yahoo: Y,
    biggo: B,
    pause: P,
) -> Result<RunSummary>
where
    Y: Fn(&'a str) -> YF,
    YF: Future<Output = Result<StockReports>>,
    B: Fn(&'a str) -> BF,
    BF: Future<Output = Result<StockReports>>,
    P: Fn() -> PF,
    PF: Future<Output = ()>,
{
    let mut summary = RunSummary {
        candidates: candidates.len(),
        ..Default::default()
    };

    for (index, stock_symbol) in candidates.iter().enumerate() {
        if summary.attempted() >= max_stocks {
            break;
        }

        let prioritized = index < priority;
        let cache_key = make_cache_key(stock_symbol);
        let skip_key = if prioritized {
            make_priority_cache_key(stock_symbol)
        } else {
            cache_key.clone()
        };
        if CLIENT
            .get_bool(&skip_key)
            .await
            .with_context(|| format!("redis get_bool failed: cache_key={skip_key}"))?
        {
            summary.skipped += 1;
            continue;
        }

        // 先寫旗標：即使這檔失敗，下一輪也不會立刻重打同一來源。
        if prioritized {
            summary.prioritized += 1;
            CLIENT
                .set(&skip_key, true, PRIORITY_TTL_SECONDS)
                .await
                .with_context(|| format!("redis set failed: cache_key={skip_key}"))?;
        }
        CLIENT
            .set(&cache_key, true, REFRESH_TTL_SECONDS)
            .await
            .with_context(|| format!("redis set failed: cache_key={cache_key}"))?;

        let yahoo_result = match yahoo(stock_symbol).await {
            Ok(reports) => save_reports(repo, stock_symbol, &reports).await,
            Err(why) => Err(why),
        };
        match yahoo_result {
            Ok(counts) => {
                summary.succeeded += 1;
                summary.rows += counts.total();
            }
            Err(why) if yahoo::dividend::is_page_not_found_error(&why) => {
                summary.not_found += 1;
                // 404 幾乎都是已下市、主檔旗標還沒更新的代號，拉長略過時間（優先旗標也一樣）。
                let keys = [Some(&cache_key), prioritized.then_some(&skip_key)];
                for key in keys.into_iter().flatten() {
                    if let Err(cache_err) = CLIENT
                        .set(key, true, yahoo::dividend::PAGE_NOT_FOUND_CACHE_TTL_SECONDS)
                        .await
                    {
                        tracing::error!(
                            "Failed to extend yahoo financial report 404 skip cache: stock_symbol={stock_symbol}, error={cache_err:#}"
                        );
                    }
                }
                tracing::warn!(
                    "skip financial report backfill because yahoo page not found (證券可能已下市): stock_symbol={stock_symbol}, error={why:#}"
                );
            }
            Err(why) => {
                pause().await;
                let fallback_result = match biggo(stock_symbol).await {
                    Ok(reports) => save_biggo_reports(repo, stock_symbol, &reports).await,
                    Err(fallback_err) => Err(fallback_err),
                };
                match fallback_result {
                    Ok(counts) => {
                        summary.fallback += 1;
                        summary.rows += counts.total();
                        tracing::warn!(
                            "financial report fell back to BigGo: stock_symbol={stock_symbol}, rows={}, yahoo_error={why:#}",
                            counts.total()
                        );
                    }
                    Err(fallback_err) => {
                        summary.failed += 1;
                        tracing::error!(
                            "financial report backfill failed: stock_symbol={stock_symbol}, error={why:#}, biggo_error={fallback_err:#}"
                        );
                    }
                }
            }
        }

        pause().await;
    }

    Ok(summary)
}
