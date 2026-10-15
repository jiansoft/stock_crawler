//! # 籌碼資料入庫
//!
//! 持股通知只保存最近一次的 Redis 快照，看不到歷史；這裡把全市場的籌碼資料存進資料庫，
//! 供日後查趨勢、對外 API 使用。與通知分開排程，停掉某個通知也不影響入庫。
//!
//! | 函式 | 資料 | 排程 | 能否回補 |
//! |------|------|------|----------|
//! | [`execute_daily`] | 三大法人買賣超＋融資融券餘額（上市、上櫃），接著更新當天的連續買賣超天數 | 週一至週五 21:30 | 可（[`backfill_daily`]） |
//! | [`execute_holder_distributions`] | 集保股權分散週摘要 | 週六 10:20 | 否，只有最新一週 |
//! | [`execute_insider_holdings`] | 董監事持股與設質 | 每月 10–28 日 20:35 | 否，只有最新月份 |
//! | [`broker_flow_records`] | 券商分點主力進出（只有持股） | 由 20:40 的主力進出通知順便寫入 | 否，只有最近一天 |

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{Datelike, Local, NaiveDate};

use crate::{
    domain::{
        chip::{
            BrokerFlowRecord, BrokerNetRecord, ChipRepository, HolderDistributionRecord,
            InsiderHoldingRecord, InstitutionalNet, MarginRecord,
        },
        quote::repository::QuoteRepository,
    },
    infra::{
        crawler::{
            fbs::broker_flow::{BrokerFlow, BrokerNet},
            mops::insider_holding::{self, InsiderHolding},
            share::{InstitutionalFlow, MarginBalance},
            tdcc::shareholding,
            tpex, twse,
        },
        database::repository::{chip::PgChipRepository, quote::PgQuoteRepository},
    },
};

/// 回補時每個交易日之間的間隔；每天對證交所、櫃買各送兩個請求。
const BACKFILL_INTERVAL: Duration = Duration::from_secs(3);

/// 單日入庫的結果。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DailySummary {
    /// 取得三大法人資料的股票數。
    pub institutional: usize,
    /// 取得融資融券資料的股票數。
    pub margin: usize,
    /// 失敗的來源數（最多 4）。
    pub failed_sources: usize,
    /// 實際新增或變更的列數。
    pub rows_written: u64,
    /// 連續買賣超天數有變的列數（與 `rows_written` 可能是同一列）。
    pub streaks_updated: u64,
}

/// 排程入口：今天的三大法人與融資融券入庫。
pub async fn execute_daily() -> Result<()> {
    let date = Local::now().date_naive();
    let summary = save_daily(&PgChipRepository::new(), date).await?;
    tracing::info!(
        "籌碼資料入庫結束: date={date}, institutional={}, margin={}, failed_sources={}, rows_written={}, streaks_updated={}",
        summary.institutional,
        summary.margin,
        summary.failed_sources,
        summary.rows_written,
        summary.streaks_updated
    );
    Ok(())
}

/// 抓指定交易日上市、上櫃的三大法人與融資融券並寫入；單一來源失敗只記錄。
///
/// 休市或尚未公布時四個來源都是空的，不寫入任何資料。四個來源全部失敗才回傳錯誤。
pub async fn save_daily(repository: &dyn ChipRepository, date: NaiveDate) -> Result<DailySummary> {
    let (listed_flows, otc_flows, listed_margins, otc_margins) = tokio::join!(
        twse::chip::visit_institutional(date),
        tpex::chip::visit_institutional(date),
        twse::chip::visit_margin(date),
        tpex::chip::visit_margin(date),
    );
    let (mut summary, flows, margins) = collect_daily(
        date,
        [listed_flows, otc_flows],
        [listed_margins, otc_margins],
    )?;
    summary.rows_written += repository.save_institutional(date, &flows).await?;
    summary.rows_written += repository.save_margin(date, &margins).await?;
    summary.streaks_updated = repository.refresh_streaks(date, date).await?;
    Ok(summary)
}

/// 從 `from` 起逐年重算連續買賣超天數到 `to`（皆含），回傳更新列數。
///
/// 每年一段、各自一個交易：每段開頭接續前一段最後一列的天數，結果與一次重算全期相同，
/// 但不會一次鎖住或改寫整張表。
pub async fn refresh_streaks_since(
    repository: &dyn ChipRepository,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<u64> {
    let mut updated = 0;
    for (start, end) in yearly_chunks(from, to) {
        let rows = repository.refresh_streaks(start, end).await?;
        tracing::info!("籌碼連續天數重算 {start}～{end}: rows_updated={rows}");
        updated += rows;
    }
    Ok(updated)
}

/// 把 `from`～`to` 依日曆年切段（皆含）；起日晚於迄日回空。
fn yearly_chunks(from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    let mut chunks = Vec::new();
    let mut start = from;
    while start <= to {
        let year_end = NaiveDate::from_ymd_opt(start.year(), 12, 31).unwrap_or(to);
        let end = year_end.min(to);
        chunks.push((start, end));
        match end.succ_opt() {
            Some(next) => start = next,
            None => break,
        }
    }
    chunks
}

/// 上市、上櫃各兩個來源的結果轉成入庫記錄；單一來源失敗只記錄並計數，四個都失敗回錯誤。
fn collect_daily(
    date: NaiveDate,
    [listed_flows, otc_flows]: [Result<HashMap<String, InstitutionalFlow>>; 2],
    [listed_margins, otc_margins]: [Result<HashMap<String, MarginBalance>>; 2],
) -> Result<(DailySummary, Vec<InstitutionalNet>, Vec<MarginRecord>)> {
    let mut summary = DailySummary::default();
    let mut flows = Vec::new();
    for (market, result) in [("上市", listed_flows), ("上櫃", otc_flows)] {
        match result {
            Ok(map) => flows.extend(institutional_records(map)),
            Err(why) => {
                summary.failed_sources += 1;
                tracing::warn!("{date} {market}三大法人抓取失敗: {why:#}");
            }
        }
    }
    let mut margins = Vec::new();
    for (market, result) in [("上市", listed_margins), ("上櫃", otc_margins)] {
        match result {
            Ok(map) => margins.extend(margin_records(map)),
            Err(why) => {
                summary.failed_sources += 1;
                tracing::warn!("{date} {market}融資融券抓取失敗: {why:#}");
            }
        }
    }
    if summary.failed_sources == 4 {
        bail!("{date} 三大法人與融資融券四個來源全部抓取失敗");
    }

    summary.institutional = flows.len();
    summary.margin = margins.len();
    Ok((summary, flows, margins))
}

/// 回補 `from`～`to`（皆含）每個交易日的三大法人與融資融券，回傳寫入列數。
///
/// 交易日取自日報價（有成交紀錄的日期）；單日失敗只記錄並繼續，重跑只會補上有差異的列。
pub async fn backfill_daily(from: NaiveDate, to: NaiveDate) -> Result<u64> {
    let mut dates: Vec<NaiveDate> = PgQuoteRepository::new()
        .fetch_market_traded_counts(from, to)
        .await?
        .into_iter()
        .map(|count| count.date)
        .collect();
    dates.sort_unstable();
    dates.dedup();

    let repository = PgChipRepository::new();
    let mut written = 0;
    for (index, date) in dates.iter().enumerate() {
        match save_daily(&repository, *date).await {
            Ok(summary) => {
                written += summary.rows_written;
                tracing::info!(
                    "籌碼回補 {date} ({}/{}): institutional={}, margin={}, failed_sources={}, rows_written={}",
                    index + 1,
                    dates.len(),
                    summary.institutional,
                    summary.margin,
                    summary.failed_sources,
                    summary.rows_written
                );
            }
            Err(why) => tracing::warn!("籌碼回補 {date} 失敗，略過: {why:#}"),
        }
        tokio::time::sleep(BACKFILL_INTERVAL).await;
    }
    // 補寫的日子已逐日更新連續天數；之後的日子要接續新的天數，從起日重算到今天。
    if let Some(&first) = dates.first() {
        written += refresh_streaks_since(&repository, first, Local::now().date_naive()).await?;
    }
    Ok(written)
}

/// 排程入口：最新一週的集保股權分散入庫。
pub async fn execute_holder_distributions() -> Result<()> {
    let distributions = shareholding::visit().await?;
    let records: Vec<HolderDistributionRecord> = distributions
        .into_iter()
        .map(|(stock_symbol, d)| HolderDistributionRecord {
            date: d.date,
            stock_symbol,
            major_holders: d.major_holders,
            major_percent: d.major_percent,
            total_holders: d.total_holders,
        })
        .collect();
    let written = PgChipRepository::new()
        .save_holder_distributions(&records)
        .await?;
    tracing::info!(
        "集保股權分散入庫結束: records={}, rows_written={written}",
        records.len()
    );
    Ok(())
}

/// 排程入口：最新月份的董監事持股與設質入庫；上市、上櫃任一邊失敗只記錄。
pub async fn execute_insider_holdings() -> Result<()> {
    let (listed, otc) = tokio::join!(
        insider_holding::visit_listed(),
        insider_holding::visit_otc()
    );
    let mut records = Vec::new();
    let mut failed = 0;
    for (market, result) in [("上市", listed), ("上櫃", otc)] {
        match result {
            Ok(holdings) => records.extend(holdings.into_iter().map(insider_record)),
            Err(why) => {
                failed += 1;
                tracing::warn!("{market}董監持股抓取失敗: {why:#}");
            }
        }
    }
    if failed == 2 {
        bail!("上市、上櫃董監持股都抓取失敗");
    }
    let written = PgChipRepository::new()
        .replace_insider_holdings(&records)
        .await?;
    tracing::info!(
        "董監持股入庫結束: records={}, rows_written={written}",
        records.len()
    );
    Ok(())
}

/// 主力進出通知抓到的資料轉成入庫格式。
pub fn broker_flow_records(flows: &[BrokerFlow]) -> Vec<BrokerFlowRecord> {
    let net = |broker: &BrokerNet| BrokerNetRecord {
        name: broker.name.clone(),
        buy: broker.buy,
        sell: broker.sell,
        net: broker.net,
        share: broker.share,
    };
    flows
        .iter()
        .map(|flow| BrokerFlowRecord {
            date: flow.date,
            stock_symbol: flow.stock_symbol.clone(),
            buy_total: flow.buy_total,
            sell_total: flow.sell_total,
            main_share: flow.main_share(),
            buyers: flow.buyers.iter().map(net).collect(),
            sellers: flow.sellers.iter().map(net).collect(),
        })
        .collect()
}

/// 寫入主力進出；失敗只記錄，不影響通知。
pub async fn save_broker_flows(flows: &[BrokerFlow]) {
    match PgChipRepository::new()
        .save_broker_flows(&broker_flow_records(flows))
        .await
    {
        Ok(written) => tracing::info!(
            "主力進出入庫結束: records={}, rows_written={written}",
            flows.len()
        ),
        Err(why) => tracing::error!("主力進出入庫失敗: {why:#}"),
    }
}

fn institutional_records(map: HashMap<String, InstitutionalFlow>) -> Vec<InstitutionalNet> {
    map.into_iter()
        .map(|(stock_symbol, flow)| InstitutionalNet {
            stock_symbol,
            foreign: flow.foreign,
            trust: flow.trust,
            dealer: flow.dealer,
        })
        .collect()
}

fn margin_records(map: HashMap<String, MarginBalance>) -> Vec<MarginRecord> {
    map.into_iter()
        .map(|(stock_symbol, margin)| MarginRecord {
            stock_symbol,
            margin_previous: margin.margin_previous,
            margin_balance: margin.margin_today,
            short_previous: margin.short_previous,
            short_balance: margin.short_today,
        })
        .collect()
}

fn insider_record(holding: InsiderHolding) -> InsiderHoldingRecord {
    InsiderHoldingRecord {
        month: holding.month,
        stock_symbol: holding.stock_symbol,
        title: holding.title,
        name: holding.name,
        shares: holding.shares,
        pledged: holding.pledged,
        related_pledged: holding.related_pledged,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 主力進出轉成入庫格式時保留合計列與各分點，主力比重由分點比重相減。
    #[test]
    fn broker_flow_records_keep_totals_and_brokers() {
        let broker = |name: &str, net, share| BrokerNet {
            name: name.to_string(),
            buy: net + 10,
            sell: 10,
            net,
            share,
        };
        let flow = BrokerFlow {
            stock_symbol: "2884".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
            buyers: vec![broker("凱基-台北", 500, dec!(6.98))],
            sellers: vec![broker("元大-總公司", 300, dec!(2.5))],
            buy_total: 500,
            sell_total: 300,
        };

        let records = broker_flow_records(&[flow]);

        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!((record.buy_total, record.sell_total), (500, 300));
        assert_eq!(record.main_share, dec!(4.48));
        assert_eq!(record.buyers[0].name, "凱基-台北");
        assert_eq!(record.sellers[0].net, 300);
    }

    /// 四個來源的結果合併成入庫記錄：失敗的來源只計數；四個全部失敗回錯誤。
    #[test]
    fn collect_daily_merges_sources_and_counts_failures() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
        let flows = |symbol: &str| {
            Ok(HashMap::from([(
                symbol.to_string(),
                InstitutionalFlow {
                    foreign: 100,
                    trust: -20,
                    dealer: 5,
                },
            )]))
        };
        let margins = || {
            Ok(HashMap::from([(
                "2330".to_string(),
                MarginBalance {
                    margin_previous: 10,
                    margin_today: 12,
                    short_previous: 3,
                    short_today: 1,
                },
            )]))
        };

        let (summary, institutional, margin) = collect_daily(
            date,
            [flows("2330"), flows("6488")],
            [margins(), Err(anyhow::anyhow!("逾時"))],
        )
        .expect("部分失敗不是錯誤");
        assert_eq!(
            summary,
            DailySummary {
                institutional: 2,
                margin: 1,
                failed_sources: 1,
                rows_written: 0,
                streaks_updated: 0
            }
        );
        assert_eq!(institutional.len(), 2);
        assert!(
            institutional
                .iter()
                .any(|net| net.stock_symbol == "6488" && net.trust == -20)
        );
        assert_eq!(margin[0].margin_balance, 12);

        fn failed<T>() -> Result<T> {
            Err(anyhow::anyhow!("封鎖"))
        }
        let err = collect_daily(date, [failed(), failed()], [failed(), failed()])
            .expect_err("四個來源都失敗");
        assert!(err.to_string().contains("四個來源全部抓取失敗"), "{err}");
    }

    /// 融資融券欄位對應：今日餘額寫到 balance、前日餘額寫到 previous。
    #[test]
    fn margin_records_map_today_and_previous() {
        let map = HashMap::from([(
            "2330".to_string(),
            MarginBalance {
                margin_previous: 10,
                margin_today: 12,
                short_previous: 3,
                short_today: 1,
            },
        )]);

        assert_eq!(
            margin_records(map),
            [MarginRecord {
                stock_symbol: "2330".to_string(),
                margin_previous: 10,
                margin_balance: 12,
                short_previous: 3,
                short_balance: 1,
            }]
        );
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
    }

    /// 逐年切段：頭尾不足一年照實際起訖，同一年只有一段，起日晚於迄日沒有任何段。
    #[test]
    fn yearly_chunks_split_by_calendar_year() {
        assert_eq!(
            yearly_chunks(ymd(2024, 3, 5), ymd(2026, 2, 1)),
            [
                (ymd(2024, 3, 5), ymd(2024, 12, 31)),
                (ymd(2025, 1, 1), ymd(2025, 12, 31)),
                (ymd(2026, 1, 1), ymd(2026, 2, 1)),
            ]
        );
        assert_eq!(
            yearly_chunks(ymd(2026, 10, 15), ymd(2026, 10, 15)),
            [(ymd(2026, 10, 15), ymd(2026, 10, 15))]
        );
        assert!(yearly_chunks(ymd(2026, 10, 16), ymd(2026, 10, 15)).is_empty());
    }

    /// 只記錄連續天數重算區間的假倉儲；其餘方法不應被呼叫。
    #[derive(Default)]
    struct StreakRecorder {
        calls: std::sync::Mutex<Vec<(NaiveDate, NaiveDate)>>,
    }

    #[async_trait::async_trait]
    impl ChipRepository for StreakRecorder {
        async fn save_institutional(&self, _: NaiveDate, _: &[InstitutionalNet]) -> Result<u64> {
            unreachable!("不應寫入三大法人")
        }

        async fn save_margin(&self, _: NaiveDate, _: &[MarginRecord]) -> Result<u64> {
            unreachable!("不應寫入融資融券")
        }

        async fn refresh_streaks(&self, from: NaiveDate, to: NaiveDate) -> Result<u64> {
            self.calls.lock().expect("測試鎖").push((from, to));
            Ok(10)
        }

        async fn save_holder_distributions(&self, _: &[HolderDistributionRecord]) -> Result<u64> {
            unreachable!("不應寫入集保")
        }

        async fn replace_insider_holdings(&self, _: &[InsiderHoldingRecord]) -> Result<u64> {
            unreachable!("不應寫入董監持股")
        }

        async fn save_broker_flows(&self, _: &[BrokerFlowRecord]) -> Result<u64> {
            unreachable!("不應寫入分點")
        }
    }

    /// 逐年依序重算並加總更新列數。
    #[tokio::test]
    async fn refresh_streaks_since_runs_each_year_in_order() {
        let repo = StreakRecorder::default();
        let updated = refresh_streaks_since(&repo, ymd(2025, 6, 1), ymd(2026, 10, 15))
            .await
            .expect("成功");
        assert_eq!(updated, 20);
        assert_eq!(
            repo.calls.lock().expect("測試鎖").clone(),
            [
                (ymd(2025, 6, 1), ymd(2025, 12, 31)),
                (ymd(2026, 1, 1), ymd(2026, 10, 15)),
            ]
        );
    }
}
