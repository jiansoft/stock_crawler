//! 以「代號 × 月份區間」回補歷史日報價。
//!
//! 與 [`super::quote`] 的分工：那支負責「某一交易日的全市場」，是排程每天走的路；
//! 這支負責「某幾檔股票的某段歷史」，補的是資料庫既有的缺口。實測 2015–2021
//! 這七年間 `00` 開頭的 ETF 在 `DailyQuotes` 一筆都沒有，逐日重跑全市場既慢
//! 又會連不缺的股票一起重抓，因此改用個股月行情：上市走 TWSE `STOCK_DAY`，
//! 上櫃（主要是債券 ETF）走櫃買中心 `tradingStock`，依股票主檔的市場別自動選擇。
//!
//! 寫入一律是「只補空位」：既有資料不覆寫也不刪除，所以中途失敗直接重跑即可。

use anyhow::{Context, Result};
use chrono::{Datelike, Months, NaiveDate};

use crate::{
    app::backfill::acl::QuoteAclMapper,
    app::backfill::port::MonthlyQuoteFetcher,
    app::calculation,
    core::declare::StockExchangeMarket,
    domain::quote::repository::QuoteRepository,
    infra::cache::SHARE,
    infra::crawler::{share::DailyQuoteDto, tpex, twse},
    infra::database::repository::quote::PgQuoteRepository,
};

/// 每次向 TWSE 要一個月資料之間的間隔。
///
/// 證交所與櫃買中心對同一來源的高頻請求會回 429；`core::util::http` 雖然有退避重試，
/// 但回補動輒兩萬次請求，主動放慢比事後重試划算。
const REQUEST_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1_200);

/// 回補的執行摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuoteHistoryBackfillSummary {
    /// 實際請求的「代號 × 月份」組合數。
    pub months_requested: usize,
    /// 來源有回傳資料的組合數。
    pub months_with_data: usize,
    /// 抓取失敗而略過的組合數。
    pub months_failed: usize,
    /// 來源回傳的報價筆數。
    pub quotes_fetched: usize,
    /// 實際新增的資料列數（已存在者不計）。
    pub rows_inserted: u64,
    /// 回補後重算均線而實際更新的日報價列數。
    pub rows_recalculated: u64,
}

/// 回補指定股票在指定月份區間的日報價。
///
/// 來源依股票主檔的市場別決定（見 [`MarketRoutedMonthlyQuoteFetcher`]），
/// 呼叫前須先 `SHARE.load()`，否則一律當上市處理。
///
/// `from` 與 `to` 只取年月，兩者皆含。單一月份抓取失敗只記錄並繼續 ——
/// 回補七年份時，因為單月暫時性失敗就中止整批並不划算，重跑一次即可補上
/// （寫入是「只補空位」，重跑不會產生重複資料）。
pub async fn execute(
    stock_symbols: &[String],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<QuoteHistoryBackfillSummary> {
    let repository = PgQuoteRepository::new();
    let fetcher = MarketRoutedMonthlyQuoteFetcher {
        listed: &twse::stock_day::TwseMonthlyQuoteFetcher,
        otc: &tpex::stock_day::TpexMonthlyQuoteFetcher,
        market_of: cached_market_id,
    };
    execute_with(&fetcher, &repository, stock_symbols, from, to).await
}

/// 依股票主檔的市場別選擇個股月行情來源：上櫃走櫃買中心，其餘走證交所。
///
/// TWSE `STOCK_DAY` 查上櫃代號只會回「查無資料」，不分流的話上櫃缺口永遠補不回來。
/// 查不到市場別的代號（快取未載入、已不在主檔）沿用證交所。
struct MarketRoutedMonthlyQuoteFetcher<'a> {
    /// 上市（與其他非上櫃）證券的來源。
    listed: &'a dyn MonthlyQuoteFetcher,
    /// 上櫃證券的來源。
    otc: &'a dyn MonthlyQuoteFetcher,
    /// 查詢代號的市場別；正式流程是 [`cached_market_id`]。
    market_of: fn(&str) -> Option<i32>,
}

#[async_trait::async_trait]
impl MonthlyQuoteFetcher for MarketRoutedMonthlyQuoteFetcher<'_> {
    async fn fetch(&self, stock_symbol: &str, month: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
        if (self.market_of)(stock_symbol) == Some(StockExchangeMarket::OverTheCounter.serial()) {
            self.otc.fetch(stock_symbol, month).await
        } else {
            self.listed.fetch(stock_symbol, month).await
        }
    }
}

/// 從股票主檔快取查市場別。
fn cached_market_id(stock_symbol: &str) -> Option<i32> {
    SHARE
        .stocks
        .read()
        .ok()?
        .get(stock_symbol)
        .map(|stock| stock.market_id())
}

/// [`execute`] 的可注入版本，供測試驗證流程本身。
///
/// 抓取端與寫入端都由呼叫端提供，測試才能在不連外部網站、不碰資料庫的情況下
/// 驗證編排行為（計數、單月失敗續跑、錯誤上拋）。
pub async fn execute_with(
    fetcher: &dyn MonthlyQuoteFetcher,
    repository: &dyn QuoteRepository,
    stock_symbols: &[String],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<QuoteHistoryBackfillSummary> {
    let months = months_between(from, to)?;
    let mut summary = QuoteHistoryBackfillSummary::default();
    // 有新增資料列的代號；補進的日子會改變之後的均線，回補完要重算。
    let mut touched: Vec<String> = Vec::new();

    for stock_symbol in stock_symbols {
        for month in &months {
            summary.months_requested += 1;

            let dtos = match fetcher.fetch(stock_symbol, *month).await {
                Ok(value) => value,
                Err(why) => {
                    summary.months_failed += 1;
                    tracing::warn!(
                        stock_symbol = stock_symbol,
                        month = %month.format("%Y-%m"),
                        error = %why,
                        "個股月行情抓取失敗，略過此月份"
                    );
                    tokio::time::sleep(REQUEST_INTERVAL).await;
                    continue;
                }
            };

            if dtos.is_empty() {
                // 該股當月尚未上市或全月無成交，屬正常情況。
                tokio::time::sleep(REQUEST_INTERVAL).await;
                continue;
            }

            summary.months_with_data += 1;
            summary.quotes_fetched += dtos.len();

            let entities: Vec<_> = dtos
                .iter()
                .map(|dto| QuoteAclMapper::from_command(&QuoteAclMapper::from_dto(dto)))
                .collect();

            let inserted = repository
                .insert_missing_daily_quotes(&entities)
                .await
                .with_context(|| {
                    format!(
                        "Failed to insert quotes for {stock_symbol} at {}",
                        month.format("%Y-%m")
                    )
                })?;
            summary.rows_inserted += inserted;
            if inserted > 0 && touched.last() != Some(stock_symbol) {
                touched.push(stock_symbol.clone());
            }

            tracing::info!(
                stock_symbol = stock_symbol,
                month = %month.format("%Y-%m"),
                fetched = dtos.len(),
                inserted = inserted,
                "個股月行情回補完成"
            );

            tokio::time::sleep(REQUEST_INTERVAL).await;
        }
    }

    // 報價已經寫進去了，重算失敗只記錄、不讓整次回補失敗——
    // 之後用 `test_recalculate_moving_averages` 補算即可，不必重抓行情。
    if let Some(first_month) = months.first() {
        match calculation::daily_quotes::recalculate_moving_averages(
            repository,
            &touched,
            *first_month,
        )
        .await
        {
            Ok(rows) => summary.rows_recalculated = rows,
            Err(why) => tracing::error!(
                symbols = touched.join(","),
                from = %first_month,
                "回補後重算均線失敗: {why:#}"
            ),
        }
    }

    tracing::info!(
        symbols = stock_symbols.len(),
        months_requested = summary.months_requested,
        months_with_data = summary.months_with_data,
        months_failed = summary.months_failed,
        quotes_fetched = summary.quotes_fetched,
        rows_inserted = summary.rows_inserted,
        rows_recalculated = summary.rows_recalculated,
        "歷史日報價回補完成"
    );

    Ok(summary)
}

/// 取得目前未下市、代號以 `00` 開頭的股票（上市／上櫃 ETF 與 ETN）。
///
/// 2015–2021 的缺口正是整類 `00` 開頭代號，逐一列舉代號既冗長又會漏；
/// 直接從股票母檔取，新掛牌的 ETF 也會自動納入。
pub async fn fetch_etf_symbols() -> Result<Vec<String>> {
    use crate::domain::registry::repository::StockRepository;

    let repository = crate::infra::database::repository::stock::PgStockRepository::new();
    let mut symbols: Vec<String> = repository
        .fetch_all_active()
        .await
        .context("Failed to fetch active stocks for ETF symbols")?
        .into_iter()
        .map(|stock| stock.symbol().0.clone())
        .filter(|symbol| symbol.starts_with("00"))
        .collect();
    symbols.sort_unstable();
    symbols.dedup();
    Ok(symbols)
}

/// 列出 `from` 到 `to`（含）之間每個月的第一天。
fn months_between(from: NaiveDate, to: NaiveDate) -> Result<Vec<NaiveDate>> {
    let start = first_day_of_month(from);
    let end = first_day_of_month(to);
    if start > end {
        anyhow::bail!("起始月份不得晚於結束月份：{start} > {end}");
    }

    let mut months = Vec::new();
    let mut cursor = start;
    while cursor <= end {
        months.push(cursor);
        cursor = cursor
            .checked_add_months(Months::new(1))
            .context("Failed to advance to the next month")?;
    }
    Ok(months)
}

/// 取得該日期所屬月份的第一天。
fn first_day_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

#[cfg(test)]
mod tests;
