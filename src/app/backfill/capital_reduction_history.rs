//! # 上櫃減資歷史缺口的候選收斂
//!
//! ## 為什麼需要這一層
//!
//! 上市減資可由 [`crate::infra::crawler::twse::capital_reduction`] 全量回補，
//! 但上櫃沒有任何可回溯的公開來源：
//!
//! | 來源 | 結論 |
//! |------|------|
//! | TPEx `bulletin/revivt` | 只回當週，`date`／`startDate` 參數一律被忽略 |
//! | TPEx OpenAPI `tpex_spendi_history` | 只有當年度，且不含價格與減資原因 |
//! | TPEx／TWSE OpenAPI 全部資料集 | 無任何減資、股本形成相關項目 |
//! | TWSE `TWTAUU` | 僅含上市（實測 225 檔代號，無任何上櫃股） |
//!
//! 因此上櫃歷史只能逐檔補。但「逐檔」不等於「逐日輪詢兩千多個交易日」——
//! 本模組先把範圍收斂成一份**很短的待辦清單**，再交給人工或未來的逐檔爬蟲。
//!
//! ## 收斂原理
//!
//! [`CagrSourceRepository::fetch_anomaly_events`] 已經在做我們要的事：找出
//! 「單日收盤價跳動超過門檻、卻沒有對應除權息、且尚未登錄成公司行動」的
//! `(代號, 日期)`。跑完上市全量回補之後，這份清單剩下的就幾乎只有
//! 上櫃減資與 ETF 分割。
//!
//! 本模組在其上再做三件事：
//!
//! 1. **只留上櫃**：上市的部分已由 TWSE 全量覆蓋，仍出現代表是分割而非減資，
//!    不屬於這條路徑。
//! 2. **同檔多日收斂成一筆待辦**：同一檔可能有數次事件，列成同一列比較好處理。
//! 3. **附上跳動幅度**：方向與幅度能幫人工判斷是減資（價格上跳）還是
//!    分割（價格下跳），以及量級是否合理。
//!
//! ## 這個模組刻意不寫資料庫
//!
//! 產生比例需要「恢復買賣參考價」，而這正是目前拿不到的東西。單憑漲跌停
//! ±10% 只能把比例框在一個區間，減資的比例又不是整數倍，框不出唯一解
//! （分割可以，因為比例是整數，這也是既有 28 筆分割能被推定出來的原因）。
//! **寧可留白也不要寫入猜測值**：錯誤的比例會讓該股整段報酬率算錯，
//! 比缺資料更難發現。
//!
//! 因此本模組的產出是一份報告，供：
//!
//! - 人工透過 backfill admin 頁面或 gRPC `save_corporate_action` 逐筆登錄；
//! - 未來接上逐檔來源時，直接當成要抓的目標清單。

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{
    core::declare::StockExchangeMarket,
    domain::performance::source::CagrSourceRepository,
    infra::{cache::SHARE, database::repository::cagr_source::PgCagrSourceRepository},
};

/// 單一待辦事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEvent {
    /// 事件發生日（價格跳動當天，等同減資的恢復買賣日）。
    pub event_date: NaiveDate,
    /// 當日收盤價相對前一交易日的變動率（%）。
    ///
    /// 正值代表價格上跳，減資通常如此（股數變少、每股價格變高）；
    /// 負值代表下跳，較可能是分割。
    pub change_percent: Decimal,
}

/// 單一股票的待辦清單。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSymbol {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱，取自快取；查不到時為空字串。
    pub name: String,
    /// 該檔所有尚未解釋的事件，依日期排序。
    pub events: Vec<PendingEvent>,
}

/// 收斂結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingReport {
    /// 掃描區間的起日。
    pub from: NaiveDate,
    /// 掃描區間的迄日。
    pub to: NaiveDate,
    /// 收斂前的原始事件數。
    pub total_events: usize,
    /// 因非上櫃而被濾掉的事件數。
    pub skipped_non_otc: usize,
    /// 待辦清單，依代號排序。
    pub symbols: Vec<PendingSymbol>,
}

impl PendingReport {
    /// 待辦的股票檔數。
    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    /// 待辦的事件總數。
    pub fn event_count(&self) -> usize {
        self.symbols.iter().map(|item| item.events.len()).sum()
    }
}

/// 掃描指定區間，產生上櫃減資的待辦清單。
///
/// 只讀不寫。建議在跑完
/// [`crate::app::backfill::capital_reduction::backfill_listed_full`] 之後執行，
/// 此時清單裡才不會混入上市的部分。
pub async fn scan(from: NaiveDate, to: NaiveDate) -> Result<PendingReport> {
    let repository = PgCagrSourceRepository::new();
    let events = repository
        .fetch_anomaly_events(from, to)
        .await
        .context("Failed to fetch anomaly events for capital reduction history")?;

    let total_events = events.len();
    let mut skipped_non_otc = 0_usize;
    let mut grouped: BTreeMap<String, (String, Vec<NaiveDate>)> = BTreeMap::new();

    for (stock_symbol, event_date) in events {
        // 快取查不到的代號（例如已下市很久、或權證之類的非股票代號）一併略過：
        // 無法確認市場別就無法確認它屬不屬於這條路徑。
        let Some(stock) = SHARE.get_stock(&stock_symbol).await else {
            skipped_non_otc += 1;
            continue;
        };

        if stock.market_id() != StockExchangeMarket::OverTheCounter.serial() {
            skipped_non_otc += 1;
            continue;
        }

        grouped
            .entry(stock_symbol)
            .or_insert_with(|| (stock.name().to_owned(), Vec::new()))
            .1
            .push(event_date);
    }

    let mut symbols = Vec::with_capacity(grouped.len());
    for (stock_symbol, (name, mut dates)) in grouped {
        dates.sort_unstable();
        dates.dedup();

        let mut events = Vec::with_capacity(dates.len());
        for event_date in dates {
            // 取不到變動率不影響待辦本身的成立，欄位留 0 即可。
            let change_percent = repository
                .fetch_single_day_change_percent(&stock_symbol, event_date)
                .await
                .unwrap_or_default()
                .unwrap_or_default();

            events.push(PendingEvent {
                event_date,
                change_percent,
            });
        }

        symbols.push(PendingSymbol {
            stock_symbol,
            name,
            events,
        });
    }

    Ok(PendingReport {
        from,
        to,
        total_events,
        skipped_non_otc,
        symbols,
    })
}

/// 把報告輸出成可讀的多行文字，供手動回補入口印出。
pub fn format_report(report: &PendingReport) -> String {
    let mut lines = Vec::new();

    lines.push(format!(
        "上櫃減資待辦：掃描 {} ~ {}，原始異常事件 {} 筆，非上櫃濾除 {} 筆，剩 {} 檔 / {} 筆事件",
        report.from,
        report.to,
        report.total_events,
        report.skipped_non_otc,
        report.symbol_count(),
        report.event_count()
    ));

    for symbol in &report.symbols {
        let detail: Vec<String> = symbol
            .events
            .iter()
            // 用 `{:.2}` 而不是 round_dp(2)：後者只會捨去多餘位數、不會補零，
            // 35.5 會印成「35.5」而 62.1234 印成「62.12」，同一份清單的小數位
            // 參差不齊，掃過去時很難比較量級。
            .map(|event| format!("{} ({:.2}%)", event.event_date, event.change_percent))
            .collect();

        lines.push(format!(
            "  {} {}：{}",
            symbol.stock_symbol,
            symbol.name,
            detail.join("、")
        ));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
    }

    fn sample_report() -> PendingReport {
        PendingReport {
            from: date(2015, 1, 1),
            to: date(2026, 9, 17),
            total_events: 120,
            skipped_non_otc: 100,
            symbols: vec![
                PendingSymbol {
                    stock_symbol: "3710".to_string(),
                    name: "連展投控".to_string(),
                    events: vec![
                        PendingEvent {
                            event_date: date(2020, 5, 4),
                            change_percent: dec!(62.1234),
                        },
                        PendingEvent {
                            event_date: date(2023, 8, 1),
                            change_percent: dec!(35.5),
                        },
                    ],
                },
                PendingSymbol {
                    stock_symbol: "8277".to_string(),
                    name: "商丞".to_string(),
                    events: vec![PendingEvent {
                        event_date: date(2019, 3, 1),
                        change_percent: dec!(-40.0),
                    }],
                },
            ],
        }
    }

    #[test]
    fn counts_symbols_and_events() {
        let report = sample_report();

        assert_eq!(report.symbol_count(), 2);
        assert_eq!(report.event_count(), 3);
    }

    #[test]
    fn empty_report_counts_zero() {
        let report = PendingReport::default();

        assert_eq!(report.symbol_count(), 0);
        assert_eq!(report.event_count(), 0);
    }

    /// 報告要能一眼看出規模與每檔的事件日期，變動率取到小數兩位。
    #[test]
    fn format_report_lists_every_event() {
        let text = format_report(&sample_report());

        assert!(text.contains("原始異常事件 120 筆"));
        assert!(text.contains("非上櫃濾除 100 筆"));
        assert!(text.contains("剩 2 檔 / 3 筆事件"));
        assert!(text.contains("3710 連展投控：2020-05-04 (62.12%)、2023-08-01 (35.50%)"));
        assert!(
            text.contains("8277 商丞：2019-03-01 (-40.00%)"),
            "小數位要補零對齊，實際輸出：{text}"
        );
    }

    /// 沒有待辦時只輸出摘要行，不應有額外空行造成誤讀。
    #[test]
    fn format_report_handles_empty_symbols() {
        let report = PendingReport {
            from: date(2015, 1, 1),
            to: date(2026, 9, 17),
            total_events: 0,
            skipped_non_otc: 0,
            symbols: Vec::new(),
        };

        let text = format_report(&report);

        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("剩 0 檔 / 0 筆事件"));
    }
}
