//! # 官方本益比與股價淨值比回補
//!
//! 用證交所 BWIBBU_d 與櫃買中心 peQryDate 的官方值，補上 `DailyQuotes` 裡為 0 的
//! 本益比與股價淨值比（只填 0、不覆寫既有值）；**上櫃本益比例外**，以官方值為準直接覆寫
//! （見 [`PePolicy`]，每日收盤流程寫入的上櫃本益比常與官方不符）。
//!
//! | 函式 | 用途 |
//! |------|------|
//! | [`execute`] | 每日 17:30、20:30 排程：補最近 [`RECENT_DAYS`] 天的缺值，再重算同一段期間的市場估值水位（`daily_market_valuation`） |
//! | [`backfill_range`] | 手動回補：逐日補一段期間（2012 年起約 3,400 個交易日、兩市場各一次請求） |
//!
//! 上市只處理「那天仍有缺值」的交易日（虧損股本益比本來就是 0，幾乎每天都會處理一次），
//! 上櫃處理所有交易日。請求之間停頓 [`REQUEST_PAUSE`]，避免被交易所擋。單日失敗只記錄、
//! 不中斷整段；全部失敗才回錯誤。

use std::{future::Future, time::Duration};

use anyhow::{Result, bail};
use chrono::{Local, NaiveDate};

use crate::{
    domain::{
        market_index::valuation::MarketValuationRepository,
        quote::official_ratio::{OfficialRatio, OfficialRatioRepository, PePolicy},
    },
    infra::{
        crawler::{tpex, twse},
        database::repository::{
            market_valuation::PgMarketValuationRepository, quote::PgQuoteRepository,
        },
    },
};

/// 排程每次回看的天數（含今天）。櫃買中心當天的本益比 17:30 常還沒公布（回 `ok` 但沒有資料），
/// 回看幾天讓晚公布或當天失敗的日期在之後的排程補上；上櫃每天都會重抓一次，請求數約 2×交易日數。
pub const RECENT_DAYS: i64 = 7;

/// 兩次請求之間的停頓。
pub const REQUEST_PAUSE: Duration = Duration::from_secs(3);

/// 回補的市場。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    /// 上市（`stock_exchange_market_id = 2`），來源 TWSE BWIBBU_d。
    Listed,
    /// 上櫃（`stock_exchange_market_id = 4`），來源 TPEx peQryDate。
    Otc,
}

impl Market {
    /// 股票主檔的市場編號。
    pub fn market_id(self) -> i32 {
        match self {
            Self::Listed => 2,
            Self::Otc => 4,
        }
    }

    /// 本益比寫入規則：上櫃以官方為準（每日收盤流程的上櫃本益比常與官方不符），上市只填 0。
    pub fn pe_policy(self) -> PePolicy {
        match self {
            Self::Listed => PePolicy::FillZeroOnly,
            Self::Otc => PePolicy::Authoritative,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Listed => "上市",
            Self::Otc => "上櫃",
        }
    }
}

/// 一次回補的結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// 處理的（市場, 交易日）組數，即請求次數。
    pub requests: usize,
    /// 抓取失敗的組數。
    pub failed: usize,
    /// 官方回傳的資料列數。
    pub fetched_rows: usize,
    /// 實際更新的日線列數。
    pub updated_rows: u64,
    /// 跟著同步的最新報價（`last_daily_quotes`）列數；日線沒有更新時不同步。
    pub synced_latest_rows: u64,
}

/// 排程入口：補最近 [`RECENT_DAYS`] 天（台北時間，含今天）兩個市場的缺值；沒有交易日時不發請求。
pub async fn execute() -> Result<()> {
    let (from, to) = recent_window(Local::now().date_naive());
    let summary = backfill_range(from, to).await?;
    tracing::info!(
        "官方本益比淨值比補值結束: requests={} failed={} fetched={} updated={} synced_latest={}",
        summary.requests,
        summary.failed,
        summary.fetched_rows,
        summary.updated_rows,
        summary.synced_latest_rows
    );
    // 上櫃本益比以官方值覆寫，補值之後才重算中位數；失敗只記錄，下一次排程會再重算同一段期間。
    match PgMarketValuationRepository::new()
        .refresh_market_valuation(from, to)
        .await
    {
        Ok(rows) => tracing::info!("重算市場估值水位結束: {from}～{to} {rows} 列"),
        Err(why) => tracing::error!("重算市場估值水位失敗: {why:#}"),
    }
    Ok(())
}

/// 排程回補的日期區間：`today` 往前共 [`RECENT_DAYS`] 天。
fn recent_window(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    (today - chrono::Duration::days(RECENT_DAYS - 1), today)
}

/// 逐日回補 `from`～`to` 兩個市場的缺值。
///
/// # Errors
///
/// 查詢缺值日期或寫入失敗，或有發請求卻全部失敗時回傳錯誤。
pub async fn backfill_range(from: NaiveDate, to: NaiveDate) -> Result<Summary> {
    run(&PgQuoteRepository::new(), from, to, fetch, || {
        tokio::time::sleep(REQUEST_PAUSE)
    })
    .await
}

/// 依市場呼叫對應的爬蟲。
async fn fetch(market: Market, date: NaiveDate) -> Result<Vec<OfficialRatio>> {
    match market {
        Market::Listed => twse::valuation_ratio::visit(date).await,
        Market::Otc => tpex::valuation_ratio::visit(date).await,
    }
}

/// 回補流程本體；`fetch` 與 `pause` 可注入，測試時換成假資料與不等待。
async fn run<F, FF, P, PF>(
    repo: &impl OfficialRatioRepository,
    from: NaiveDate,
    to: NaiveDate,
    fetch: F,
    pause: P,
) -> Result<Summary>
where
    F: Fn(Market, NaiveDate) -> FF,
    FF: Future<Output = Result<Vec<OfficialRatio>>>,
    P: Fn() -> PF,
    PF: Future<Output = ()>,
{
    let mut summary = Summary::default();
    for market in [Market::Listed, Market::Otc] {
        let dates = repo
            .fetch_dates_to_update(market.market_id(), from, to, market.pe_policy())
            .await?;
        tracing::info!(
            "官方本益比淨值比回補開始: market={} dates={} from={from} to={to}",
            market.label(),
            dates.len()
        );
        for (done, date) in dates.iter().enumerate() {
            if summary.requests > 0 {
                pause().await;
            }
            summary.requests += 1;
            match fetch(market, *date).await {
                Ok(ratios) => {
                    summary.fetched_rows += ratios.len();
                    summary.updated_rows += repo
                        .fill_official_ratios(*date, &ratios, market.pe_policy())
                        .await?;
                }
                Err(why) => {
                    summary.failed += 1;
                    tracing::warn!(
                        "官方本益比淨值比抓取失敗: market={} date={date} error={why:#}",
                        market.label()
                    );
                }
            }
            if (done + 1) % 50 == 0 {
                tracing::info!(
                    "官方本益比淨值比回補進度: market={} {}/{} 最新 {date} updated={}",
                    market.label(),
                    done + 1,
                    dates.len(),
                    summary.updated_rows
                );
            }
        }
    }
    if summary.requests > 0 && summary.failed == summary.requests {
        bail!(
            "official valuation ratio backfill failed for all {} requests",
            summary.requests
        );
    }
    if summary.updated_rows > 0 {
        summary.synced_latest_rows = repo.sync_latest_quotes().await?;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use anyhow::anyhow;
    use async_trait::async_trait;
    use rust_decimal_macros::dec;

    use super::*;

    /// 依市場回傳固定缺值日期、記下寫入內容的假倉儲。
    #[derive(Default)]
    struct FakeRepo {
        listed: Vec<NaiveDate>,
        otc: Vec<NaiveDate>,
        filled: Mutex<Vec<(NaiveDate, usize)>>,
        synced: Mutex<usize>,
    }

    #[async_trait]
    impl OfficialRatioRepository for FakeRepo {
        async fn fetch_dates_to_update(
            &self,
            market_id: i32,
            _from: NaiveDate,
            _to: NaiveDate,
            policy: PePolicy,
        ) -> Result<Vec<NaiveDate>> {
            assert_eq!(
                policy,
                if market_id == 2 {
                    PePolicy::FillZeroOnly
                } else {
                    PePolicy::Authoritative
                }
            );
            Ok(if market_id == 2 {
                self.listed.clone()
            } else {
                self.otc.clone()
            })
        }

        async fn fill_official_ratios(
            &self,
            date: NaiveDate,
            ratios: &[OfficialRatio],
            _policy: PePolicy,
        ) -> Result<u64> {
            self.filled.lock().unwrap().push((date, ratios.len()));
            Ok(ratios.len() as u64)
        }

        async fn sync_latest_quotes(&self) -> Result<u64> {
            *self.synced.lock().unwrap() += 1;
            Ok(3)
        }
    }

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2020, 6, day).expect("有效日期")
    }

    fn ratio(symbol: &str) -> OfficialRatio {
        OfficialRatio {
            stock_symbol: symbol.to_owned(),
            price_earning_ratio: Some(dec!(10)),
            price_to_book_ratio: None,
        }
    }

    /// 兩市場各自的缺值日都抓一次；單日失敗不中斷；請求之間才停頓。
    #[tokio::test]
    async fn run_visits_missing_dates_and_tolerates_failures() {
        let repo = FakeRepo {
            listed: vec![day(29), day(30)],
            otc: vec![day(30)],
            ..Default::default()
        };
        let pauses = Mutex::new(0);
        let summary = run(
            &repo,
            day(1),
            day(30),
            |market, date| async move {
                match (market, date == day(29)) {
                    (Market::Listed, true) => Err(anyhow!("逾時")),
                    (Market::Listed, false) => Ok(vec![ratio("1101"), ratio("2330")]),
                    (Market::Otc, _) => Ok(vec![ratio("6488")]),
                }
            },
            || {
                *pauses.lock().unwrap() += 1;
                async {}
            },
        )
        .await
        .expect("部分失敗不是錯誤");
        assert_eq!(
            summary,
            Summary {
                requests: 3,
                failed: 1,
                fetched_rows: 3,
                updated_rows: 3,
                synced_latest_rows: 3
            }
        );
        assert_eq!(
            *repo.synced.lock().unwrap(),
            1,
            "有更新才同步最新報價，且只同步一次"
        );
        assert_eq!(*pauses.lock().unwrap(), 2);
        assert_eq!(
            *repo.filled.lock().unwrap(),
            vec![(day(30), 2), (day(30), 1)]
        );
    }

    /// 全部失敗回錯誤；沒有缺值日不發請求、也不算失敗。
    #[tokio::test]
    async fn run_fails_only_when_every_request_fails() {
        let repo = FakeRepo {
            listed: vec![day(30)],
            ..Default::default()
        };
        let result = run(
            &repo,
            day(1),
            day(30),
            |_, _| async { Err(anyhow!("封鎖")) },
            || async {},
        )
        .await;
        assert!(result.is_err());

        let idle_repo = FakeRepo::default();
        let idle = run(
            &idle_repo,
            day(1),
            day(30),
            |_, _| async { Ok(Vec::new()) },
            || async {},
        )
        .await
        .expect("沒有缺值不是錯誤");
        assert_eq!(idle, Summary::default());
        assert_eq!(*idle_repo.synced.lock().unwrap(), 0, "沒有更新不同步");
        assert_eq!(
            (Market::Listed.market_id(), Market::Otc.market_id()),
            (2, 4)
        );
    }

    /// 排程回看區間含今天共 7 天。
    #[test]
    fn recent_window_covers_seven_days() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
        assert_eq!(
            recent_window(today),
            (
                NaiveDate::from_ymd_opt(2026, 10, 6).expect("有效日期"),
                today
            )
        );
    }
}
