//! # 籌碼資料入庫
//!
//! 持股通知只保存最近一次的 Redis 快照，看不到歷史；這裡把全市場的籌碼資料存進資料庫，
//! 供日後查趨勢、對外 API 使用。與通知分開排程，停掉某個通知也不影響入庫。
//!
//! | 函式 | 資料 | 排程 | 能否回補 |
//! |------|------|------|----------|
//! | [`execute_daily`] | 三大法人買賣超＋融資融券餘額（上市、上櫃） | 週一至週五 21:30 | 可（[`backfill_daily`]） |
//! | [`execute_holder_distributions`] | 集保股權分散週摘要 | 週六 10:20 | 否，只有最新一週 |
//! | [`execute_insider_holdings`] | 董監事持股與設質 | 每月 10–28 日 20:35 | 否，只有最新月份 |
//! | [`broker_flow_records`] | 券商分點主力進出（只有持股） | 由 20:40 的主力進出通知順便寫入 | 否，只有最近一天 |

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{Local, NaiveDate};

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
}

/// 排程入口：今天的三大法人與融資融券入庫。
pub async fn execute_daily() -> Result<()> {
    let date = Local::now().date_naive();
    let summary = save_daily(&PgChipRepository::new(), date).await?;
    tracing::info!(
        "籌碼資料入庫結束: date={date}, institutional={}, margin={}, failed_sources={}, rows_written={}",
        summary.institutional,
        summary.margin,
        summary.failed_sources,
        summary.rows_written
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
    summary.rows_written += repository.save_institutional(date, &flows).await?;
    summary.rows_written += repository.save_margin(date, &margins).await?;
    Ok(summary)
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
}
