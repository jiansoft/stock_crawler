//! 收盤行情完整性：上市或上櫃有成交的檔數偏少就重跑收盤匯總並告警，臨時停市不告警。

use super::*;

/// 收盤資料完整性檢查回看的天數，涵蓋春節等長假，確保至少有前幾個交易日可比較。
pub(super) const COVERAGE_LOOKBACK_DAYS: i64 = 14;

/// 單一市場當天有成交的檔數低於近期交易日最大值的這個百分比，就視為資料不完整。
///
/// 正常交易日有成交的檔數很穩定（上市約 1,000、上櫃約 880），整個市場沒抓到時會掉到 0，
/// 遠低於門檻；冷門股沒成交造成的波動只有數十檔。
pub(super) const COVERAGE_MIN_PERCENT: i64 = 50;

/// 收盤資料不完整的市場。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shortfall {
    /// 市場名稱。
    pub(crate) market: &'static str,
    /// 當天有成交的檔數。
    pub(crate) traded: i64,
    /// 近期交易日有成交檔數的最大值。
    pub(crate) baseline: i64,
}

/// [`check_quotes`] 的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QuoteCheck {
    /// 收盤資料完整，不用重跑。
    Complete,
    /// 已重跑收盤匯總，要發的告警（標題、內容）。
    Rerun(&'static str, String),
    /// 判定為假日表未列的臨時停市，不告警，後續檢查都不用做。
    UnscheduledClosure,
}

/// 上市或上櫃有成交的檔數偏少就以 `rerun`（正式為 [`aggregate`]）重跑收盤匯總並告警。
pub(super) async fn ensure_quotes<Rerun, RerunFut>(
    date: NaiveDate,
    rerun: Rerun,
) -> Result<QuoteCheck>
where
    Rerun: FnOnce() -> RerunFut,
    RerunFut: std::future::Future<Output = Result<()>>,
{
    let repo = PgQuoteRepository::new();
    let from = date - chrono::Duration::days(COVERAGE_LOOKBACK_DAYS);
    let check = check_quotes(
        date,
        || repo.fetch_market_traded_counts(from, date),
        rerun,
        || async {
            let counts = repo.count_after_hours(&[date]).await?;
            Ok(counts.first().map_or(0, |counts| counts.index))
        },
    )
    .await?;
    if let QuoteCheck::Rerun(title, body) = &check {
        alert::send_alert(title, body).await;
    }
    Ok(check)
}

/// [`ensure_quotes`] 的本體：以 `traded_counts` 查各市場有成交的檔數，不完整就以 `rerun` 重跑收盤
/// 匯總，重跑後兩個市場都沒有資料且 `index_count`（當天加權指數筆數）為 0 時判定臨時停市。
/// 查詢與重跑可替換以便測試。
pub(crate) async fn check_quotes<Counts, CountsFut, Rerun, RerunFut, Index, IndexFut>(
    date: NaiveDate,
    traded_counts: Counts,
    rerun: Rerun,
    index_count: Index,
) -> Result<QuoteCheck>
where
    Counts: Fn() -> CountsFut,
    CountsFut: std::future::Future<Output = Result<Vec<MarketTradedCount>>>,
    Rerun: FnOnce() -> RerunFut,
    RerunFut: std::future::Future<Output = Result<()>>,
    Index: FnOnce() -> IndexFut,
    IndexFut: std::future::Future<Output = Result<i64>>,
{
    let before = shortfalls(date, &traded_counts().await?);
    if before.is_empty() {
        tracing::info!("{date} 收盤資料完整");
        return Ok(QuoteCheck::Complete);
    }

    tracing::warn!("{date} 收盤資料不完整，重跑收盤匯總: {before:?}");
    let rerun = rerun().await;
    if let Err(why) = &rerun {
        tracing::error!("{date} 重跑收盤匯總失敗: {why:#}");
    }
    let after = shortfalls(date, &traded_counts().await?);
    if rerun.is_ok() && is_unscheduled_closure(&after, index_count().await?) {
        tracing::info!(
            "{date} 上市、上櫃與大盤指數重跑後仍都沒有資料，判定為臨時停市（交易所假日表未列），不告警"
        );
        return Ok(QuoteCheck::UnscheduledClosure);
    }

    let title = if after.is_empty() {
        "收盤資料已重抓補齊"
    } else {
        "收盤資料仍不完整"
    };
    let error = rerun.err().map(|why| format!("{why:#}"));
    Ok(QuoteCheck::Rerun(
        title,
        report(date, &before, error.as_deref(), &after),
    ))
}

/// 假日表沒列、但交易所實際沒有開市（颱風等臨時停市，例如 2026-10-23）。
///
/// 只有上市、上櫃**都**沒有任何成交，且當天也沒有加權指數才成立。兩個交易所各自發布，
/// 同時晚到 17:00／19:00 還沒有資料的機會極低；只要有一邊有資料（例如 TWSE 晚發布），
/// 仍照原本流程重跑並告警，不放寬晚發布的保護。
pub(crate) fn is_unscheduled_closure(after: &[Shortfall], index_count: i64) -> bool {
    index_count == 0
        && ["上市", "上櫃"].iter().all(|market| {
            after
                .iter()
                .any(|shortfall| shortfall.market == *market && shortfall.traded == 0)
        })
}

/// 比較當天與近期交易日各市場有成交的檔數，列出不完整的市場。
pub(crate) fn shortfalls(date: NaiveDate, counts: &[MarketTradedCount]) -> Vec<Shortfall> {
    [(2, "上市"), (4, "上櫃")]
        .into_iter()
        .filter_map(|(market_id, market)| {
            let in_market = counts.iter().filter(|count| count.market_id == market_id);
            let traded = in_market
                .clone()
                .filter(|count| count.date == date)
                .map(|count| count.traded)
                .sum::<i64>();
            let baseline = in_market
                .filter(|count| count.date < date)
                .map(|count| count.traded)
                .max()
                .unwrap_or(0);
            let short = traded == 0 || traded * 100 < baseline * COVERAGE_MIN_PERCENT;
            short.then_some(Shortfall {
                market,
                traded,
                baseline,
            })
        })
        .collect()
}

/// 告警內容：檢查時缺哪些市場、重跑是否成功、重跑後的狀況。
pub(super) fn report(
    date: NaiveDate,
    before: &[Shortfall],
    rerun_error: Option<&str>,
    after: &[Shortfall],
) -> String {
    let describe = |shortfalls: &[Shortfall]| {
        shortfalls
            .iter()
            .map(|shortfall| {
                format!(
                    "{} {} 檔有成交（近期 {} 檔）",
                    shortfall.market, shortfall.traded, shortfall.baseline
                )
            })
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = vec![format!("{date} 檢查時：{}", describe(before))];
    lines.push(match rerun_error {
        Some(why) => format!("重跑收盤匯總失敗：{why}"),
        None => "已重跑收盤匯總".to_string(),
    });
    lines.push(if after.is_empty() {
        "重跑後兩個市場都已完整".to_string()
    } else {
        format!(
            "重跑後仍不完整：{}；請確認來源是否已公布，必要時手動回補當日行情",
            describe(after)
        )
    });
    lines.join("\n")
}
