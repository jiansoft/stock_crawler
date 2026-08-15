//! 以「代號 × 月份區間」回補歷史日報價。
//!
//! 與 [`super::quote`] 的分工：那支負責「某一交易日的全市場」，是排程每天走的路；
//! 這支負責「某幾檔股票的某段歷史」，補的是資料庫既有的缺口。實測 2015–2021
//! 這七年間 `00` 開頭的 ETF 在 `DailyQuotes` 一筆都沒有，逐日重跑全市場既慢
//! 又會連不缺的股票一起重抓，因此改用 TWSE 的個股月行情（`STOCK_DAY`）。
//!
//! 寫入一律是「只補空位」：既有資料不覆寫也不刪除，所以中途失敗直接重跑即可。

use anyhow::{Context, Result};
use chrono::{Datelike, Months, NaiveDate};

use crate::{
    app::backfill::acl::QuoteAclMapper, app::backfill::port::MonthlyQuoteFetcher,
    domain::quote::repository::QuoteRepository, infra::crawler::twse,
    infra::database::repository::quote::PgQuoteRepository,
};

/// 每次向 TWSE 要一個月資料之間的間隔。
///
/// 證交所對同一來源的高頻請求會回 429；`core::util::http` 雖然有退避重試，
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
}

/// 回補指定股票在指定月份區間的日報價。
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
    let fetcher = twse::stock_day::TwseMonthlyQuoteFetcher;
    execute_with(&fetcher, &repository, stock_symbols, from, to).await
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

    tracing::info!(
        symbols = stock_symbols.len(),
        months_requested = summary.months_requested,
        months_with_data = summary.months_with_data,
        months_failed = summary.months_failed,
        quotes_fetched = summary.quotes_fetched,
        rows_inserted = summary.rows_inserted,
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
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use rust_decimal::Decimal;

    use super::*;
    use crate::domain::quote::test_double::CountingQuoteRepository;
    use crate::infra::crawler::share::DailyQuoteDto;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
    }

    /// 依「代號 × 月份」給預先安排好回應的假抓取端。
    ///
    /// 未安排的組合視為該月無資料（回傳空陣列），與 TWSE 查無資料時的行為一致。
    #[derive(Default)]
    struct StubFetcher {
        /// `(代號, 月份)` → 該月要回傳的每日筆數。
        responses: HashMap<(String, NaiveDate), usize>,
        /// 一律回傳錯誤的組合，用來模擬單月抓取失敗。
        failures: Vec<(String, NaiveDate)>,
        /// 實際被請求過的組合，依呼叫順序記錄。
        requested: Mutex<Vec<(String, NaiveDate)>>,
    }

    impl StubFetcher {
        /// 安排某代號某月回傳 `count` 筆報價。
        fn with_quotes(mut self, symbol: &str, month: NaiveDate, count: usize) -> Self {
            self.responses.insert((symbol.to_string(), month), count);
            self
        }

        /// 安排某代號某月抓取失敗。
        fn with_failure(mut self, symbol: &str, month: NaiveDate) -> Self {
            self.failures.push((symbol.to_string(), month));
            self
        }

        /// 取得實際請求過的組合。
        fn requested(&self) -> Vec<(String, NaiveDate)> {
            self.requested.lock().expect("測試鎖不應中毒").clone()
        }
    }

    #[async_trait::async_trait]
    impl MonthlyQuoteFetcher for StubFetcher {
        async fn fetch(&self, stock_symbol: &str, month: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
            let key = (stock_symbol.to_string(), month);
            self.requested
                .lock()
                .expect("測試鎖不應中毒")
                .push(key.clone());

            if self.failures.contains(&key) {
                anyhow::bail!("假抓取端：{stock_symbol} {} 失敗", month.format("%Y-%m"));
            }

            let count = self.responses.get(&key).copied().unwrap_or(0);
            Ok((0..count)
                .map(|i| {
                    let day = u32::try_from(i).expect("測試筆數不會溢位") + 1;
                    let quote_date = month.with_day(day).expect("測試日期應合法");
                    let mut dto = DailyQuoteDto::new(stock_symbol.to_string(), quote_date);
                    dto.closing_price = Decimal::from(10);
                    dto
                })
                .collect())
        }
    }

    /// 沒有代號就沒有工作：不打外部網站，也不碰倉儲。
    #[tokio::test]
    async fn execute_with_does_nothing_for_an_empty_symbol_list() {
        let fetcher = StubFetcher::default();
        let repository = CountingQuoteRepository::default();

        let summary = execute_with(
            &fetcher,
            &repository,
            &[],
            date(2015, 1, 1),
            date(2021, 12, 31),
        )
        .await
        .expect("空代號清單應成功");

        assert_eq!(summary, QuoteHistoryBackfillSummary::default());
        assert!(fetcher.requested().is_empty());
        assert_eq!(repository.insert_calls(), 0);
    }

    /// 區間反了要在送出任何請求前就失敗。
    #[tokio::test]
    async fn execute_with_rejects_a_reversed_range_before_any_request() {
        let fetcher = StubFetcher::default();
        let repository = CountingQuoteRepository::default();

        let err = execute_with(
            &fetcher,
            &repository,
            &["0050".to_string()],
            date(2022, 1, 1),
            date(2021, 12, 1),
        )
        .await
        .expect_err("反向區間應失敗");

        assert!(err.to_string().contains("起始月份"), "錯誤訊息：{err}");
        assert!(fetcher.requested().is_empty());
        assert_eq!(repository.insert_calls(), 0);
    }

    /// 逐一走過每個「代號 × 月份」，並把各項計數正確累加。
    ///
    /// `start_paused` 讓 Tokio 的虛擬時鐘自動快轉，測試不必真的等每次請求
    /// 之間的 1.2 秒間隔。
    #[tokio::test(start_paused = true)]
    async fn execute_with_walks_every_symbol_month_and_accumulates_counters() {
        let jan = date(2021, 1, 1);
        let feb = date(2021, 2, 1);
        // 0050 兩個月都有資料；0056 只有一月有，二月無成交。
        let fetcher = StubFetcher::default()
            .with_quotes("0050", jan, 3)
            .with_quotes("0050", feb, 2)
            .with_quotes("0056", jan, 1);
        let repository = CountingQuoteRepository::default();

        let summary = execute_with(
            &fetcher,
            &repository,
            &["0050".to_string(), "0056".to_string()],
            date(2021, 1, 15),
            date(2021, 2, 20),
        )
        .await
        .expect("回補應成功");

        assert_eq!(
            summary,
            QuoteHistoryBackfillSummary {
                months_requested: 4,
                months_with_data: 3,
                months_failed: 0,
                quotes_fetched: 6,
                rows_inserted: 6,
            }
        );

        // 兩檔各兩個月，順序為「代號外層、月份內層」。
        assert_eq!(
            fetcher.requested(),
            vec![
                ("0050".to_string(), jan),
                ("0050".to_string(), feb),
                ("0056".to_string(), jan),
                ("0056".to_string(), feb),
            ]
        );
        // 無資料的月份不該產生寫入呼叫。
        assert_eq!(repository.insert_calls(), 3);
        assert_eq!(repository.inserted_len(), 6);
    }

    /// 單一月份抓取失敗只記錄並繼續，不讓整批回補中止。
    ///
    /// 這是「補七年可以直接重跑」的前提：中間某個月暫時性失敗時，
    /// 其餘月份仍須完成，否則每次重跑都會卡在同一個地方。
    #[tokio::test(start_paused = true)]
    async fn execute_with_skips_a_failed_month_and_keeps_going() {
        let jan = date(2021, 1, 1);
        let feb = date(2021, 2, 1);
        let mar = date(2021, 3, 1);
        let fetcher = StubFetcher::default()
            .with_quotes("0050", jan, 2)
            .with_failure("0050", feb)
            .with_quotes("0050", mar, 1);
        let repository = CountingQuoteRepository::default();

        let summary = execute_with(
            &fetcher,
            &repository,
            &["0050".to_string()],
            jan,
            date(2021, 3, 31),
        )
        .await
        .expect("單月失敗不應讓整批失敗");

        assert_eq!(
            summary,
            QuoteHistoryBackfillSummary {
                months_requested: 3,
                months_with_data: 2,
                months_failed: 1,
                quotes_fetched: 3,
                rows_inserted: 3,
            }
        );
        // 失敗的二月之後，三月仍然被請求。
        assert_eq!(fetcher.requested().len(), 3);
        assert_eq!(repository.inserted_len(), 3);
    }

    /// 寫入失敗與抓取失敗不同：資料庫出問題必須整批中止並帶出代號與月份。
    #[tokio::test(start_paused = true)]
    async fn execute_with_aborts_when_the_repository_fails() {
        let jan = date(2021, 1, 1);
        let fetcher = StubFetcher::default()
            .with_quotes("0050", jan, 2)
            .with_quotes("0050", date(2021, 2, 1), 2);
        let repository = CountingQuoteRepository::failing();

        let err = execute_with(
            &fetcher,
            &repository,
            &["0050".to_string()],
            jan,
            date(2021, 2, 28),
        )
        .await
        .expect_err("倉儲失敗應上拋");

        let message = format!("{err:#}");
        assert!(
            message.contains("0050") && message.contains("2021-01"),
            "錯誤應指出是哪一檔的哪個月：{message}"
        );
        // 第一個月就中止，第二個月不該再被請求。
        assert_eq!(fetcher.requested().len(), 1);
    }

    /// 來源 DTO 會經過 ACL 轉成領域實體後才寫入，代號與日期不可在轉換中遺失。
    #[tokio::test(start_paused = true)]
    async fn execute_with_maps_dtos_through_the_acl_before_writing() {
        let jan = date(2021, 1, 1);
        let fetcher = StubFetcher::default().with_quotes("0050", jan, 2);
        let repository = CountingQuoteRepository::default();

        execute_with(&fetcher, &repository, &["0050".to_string()], jan, jan)
            .await
            .expect("回補應成功");

        assert_eq!(
            repository.inserted_keys(),
            vec![
                ("0050".to_string(), date(2021, 1, 1)),
                ("0050".to_string(), date(2021, 1, 2)),
            ]
        );
    }

    #[test]
    fn months_between_is_inclusive_on_both_ends() {
        let months = months_between(date(2021, 11, 15), date(2022, 2, 3)).expect("應成功");

        assert_eq!(
            months,
            vec![
                date(2021, 11, 1),
                date(2021, 12, 1),
                date(2022, 1, 1),
                date(2022, 2, 1),
            ]
        );
    }

    #[test]
    fn months_between_handles_a_single_month() {
        let months = months_between(date(2021, 8, 2), date(2021, 8, 31)).expect("應成功");
        assert_eq!(months, vec![date(2021, 8, 1)]);
    }

    #[test]
    fn months_between_spans_seven_years_of_the_known_gap() {
        // 2015-01 ~ 2021-12 共 84 個月，這正是 ETF 缺口的長度。
        let months = months_between(date(2015, 1, 1), date(2021, 12, 31)).expect("應成功");
        assert_eq!(months.len(), 84);
        assert_eq!(months.first(), Some(&date(2015, 1, 1)));
        assert_eq!(months.last(), Some(&date(2021, 12, 1)));
    }

    #[test]
    fn months_between_rejects_a_reversed_range() {
        assert!(months_between(date(2022, 1, 1), date(2021, 12, 1)).is_err());
    }
}
