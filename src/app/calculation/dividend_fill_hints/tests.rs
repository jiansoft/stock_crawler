//! 每檔股票的歷年填息提示：近 5 年填息次數與平均天數；沒有自身紀錄時改用同現金殖利率組的的測試。

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rust_decimal_macros::dec;

use super::*;
use crate::domain::dividend::fill::{DailyClose, ExRightEvent};
use crate::domain::dividend::fill_stats::FillSample;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("日期應合法")
}

/// 2330 9 月除息第 2 天填息；9999 查詢失敗；其他代號沒有除權息。
struct FakeFills;

#[async_trait]
impl DividendFillQuery for FakeFills {
    async fn fetch_latest_quote_date(&self, _symbol: &str) -> Result<Option<NaiveDate>> {
        unreachable!("不需要最新收盤日")
    }

    async fn fetch_ex_right_events(&self, symbol: &str) -> Result<Vec<ExRightEvent>> {
        match symbol {
            "2330" => Ok(vec![ExRightEvent {
                ex_date: day(9, 15),
                cash_dividend: dec!(5),
                stock_dividend: Decimal::ZERO,
            }]),
            "9999" => Err(anyhow!("db down")),
            _ => Ok(Vec::new()),
        }
    }

    async fn fetch_daily_closes(
        &self,
        _symbol: &str,
        _since: NaiveDate,
    ) -> Result<Vec<DailyClose>> {
        let close = |date, close| DailyClose { date, close };
        Ok(vec![
            close(day(9, 12), dec!(100)),
            close(day(9, 15), dec!(97)),
            close(day(9, 16), dec!(101)),
        ])
    }
}

fn stock(symbol: &str, cash: Decimal, closing_price: Decimal) -> StockDividendInfo {
    StockDividendInfo {
        stock_symbol: symbol.to_owned(),
        name: symbol.to_owned(),
        stock_industry_id: 1,
        cash_dividend: cash,
        stock_dividend: Decimal::ZERO,
        sum: cash,
        closing_price,
        dividend_yield: Decimal::ZERO,
        cash_dividend_yield: Decimal::ZERO,
        is_cash_ex_dividend_on_date: true,
        is_stock_ex_dividend_on_date: false,
        quarter: String::new(),
    }
}

/// 有自身紀錄附次數與平均；沒有時附同殖利率組中位數；都沒有時不附；查詢失敗的略過、重複代號只查一次。
#[tokio::test]
async fn text_prefers_own_history_then_peer_median() {
    let history = load_history(
        &FakeFills,
        &["2330", "2330", "1101", "9999"].map(str::to_owned),
        day(11, 2),
    )
    .await;
    assert_eq!(history.len(), 2, "9999 查詢失敗略過");
    let hints = FillHints {
        history,
        peers: bucket_stats(&[FillSample {
            bucket: YieldBucket::From3To5,
            trading_days_to_fill: Some(13),
            trading_days_observed: 120,
        }]),
    };
    assert_eq!(
        hints.text(&stock("2330", dec!(5), dec!(500))).as_deref(),
        Some("近 5 年填息 1／1 次、平均 2 日")
    );
    assert_eq!(
        hints.text(&stock("1101", dec!(1), dec!(25))).as_deref(),
        Some("同殖利率組（3～5%）填息中位數 13 日"),
        "1101 沒有除權息紀錄，1 ÷ 25 = 4%"
    );
    assert_eq!(
        hints.text(&stock("1101", dec!(1), dec!(10))),
        None,
        "10% 組沒有基準"
    );
    assert_eq!(hints.text(&stock("9999", Decimal::ZERO, dec!(10))), None);
}
