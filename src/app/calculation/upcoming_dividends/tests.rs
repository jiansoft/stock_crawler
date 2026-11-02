//! 持股未來除權息：股數合併、逐日查詢與單日失敗略過、只取當天除息／除權、排序與金額換算。

use std::sync::Mutex;

use anyhow::anyhow;
use rust_decimal_macros::dec;

use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期")
}

fn info(symbol: &str, cash: Decimal, stock: Decimal) -> StockDividendInfo {
    StockDividendInfo {
        stock_symbol: symbol.to_owned(),
        name: format!("名{symbol}"),
        stock_industry_id: 0,
        cash_dividend: cash,
        stock_dividend: stock,
        sum: cash + stock,
        closing_price: dec!(100),
        dividend_yield: Decimal::ZERO,
        cash_dividend_yield: Decimal::ZERO,
        is_cash_ex_dividend_on_date: cash > Decimal::ZERO,
        is_stock_ex_dividend_on_date: stock > Decimal::ZERO,
        quarter: String::new(),
    }
}

fn shares(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
    total_shares(pairs.iter().map(|(symbol, n)| ((*symbol).to_owned(), *n)))
}

/// 依日期回傳固定除權息；`fail_on` 那天查詢失敗；記錄查過哪些日期。
#[derive(Default)]
struct FakeDays {
    by_day: Vec<(NaiveDate, StockDividendInfo)>,
    fail_on: Option<NaiveDate>,
    asked: Mutex<Vec<NaiveDate>>,
}

#[async_trait]
impl ExDividendDayQuery for FakeDays {
    async fn fetch_on(&self, date: NaiveDate) -> Result<Vec<StockDividendInfo>> {
        self.asked.lock().expect("未中毒").push(date);
        if self.fail_on == Some(date) {
            return Err(anyhow!("dividend down"));
        }
        Ok(self
            .by_day
            .iter()
            .filter(|(d, _)| *d == date)
            .map(|(_, stock)| stock.clone())
            .collect())
    }
}

/// 同一代號分次買進、不同會員的股數合併。
#[test]
fn total_shares_merges_lots() {
    assert_eq!(
        shares(&[("2330", 100), ("0050", 50), ("2330", 400)]),
        BTreeMap::from([("0050".to_owned(), 50), ("2330".to_owned(), 500)])
    );
}

/// 逐日查 from～to（含），只留持股；單日失敗只少那天；依日期、代號排序；沒有持股不查。
#[tokio::test]
async fn upcoming_dividends_filters_holdings_and_skips_failed_days() {
    let mut only_stock_today = info("2412", dec!(4), dec!(1));
    only_stock_today.is_cash_ex_dividend_on_date = false;
    let query = FakeDays {
        by_day: vec![
            (day(21), info("2330", dec!(5), Decimal::ZERO)),
            (day(19), only_stock_today),
            (day(19), info("1101", dec!(1), Decimal::ZERO)),
            (day(20), info("0056", dec!(1), Decimal::ZERO)),
            (day(26), info("2330", dec!(5), Decimal::ZERO)),
        ],
        fail_on: Some(day(20)),
        ..Default::default()
    };
    let held = shares(&[("2330", 1000), ("2412", 2000), ("0056", 10)]);
    let items = upcoming_dividends(&query, &held, day(19), day(23)).await;
    assert_eq!(
        items,
        vec![
            UpcomingDividend {
                ex_date: day(19),
                stock_symbol: "2412".to_owned(),
                name: "名2412".to_owned(),
                cash_dividend: Decimal::ZERO,
                stock_dividend: dec!(1),
                closing_price: dec!(100),
                shares: 2000,
            },
            UpcomingDividend {
                ex_date: day(21),
                stock_symbol: "2330".to_owned(),
                name: "名2330".to_owned(),
                cash_dividend: dec!(5),
                stock_dividend: Decimal::ZERO,
                closing_price: dec!(100),
                shares: 1000,
            },
        ],
        "1101 不是持股、0056 那天查詢失敗、10-26 在期間外、2412 這天只除權"
    );
    assert_eq!(
        *query.asked.lock().expect("未中毒"),
        vec![day(19), day(20), day(21), day(22), day(23)]
    );

    let empty = FakeDays::default();
    assert!(
        upcoming_dividends(&empty, &BTreeMap::new(), day(19), day(23))
            .await
            .is_empty()
    );
    assert!(
        empty.asked.lock().expect("未中毒").is_empty(),
        "沒有持股不查"
    );
}

/// 這天既不除息也不除權（例如另一天除權）的列不算。
#[tokio::test]
async fn upcoming_dividends_skip_rows_without_event_that_day() {
    let mut neither = info("2330", dec!(5), Decimal::ZERO);
    neither.is_cash_ex_dividend_on_date = false;
    let query = FakeDays {
        by_day: vec![(day(19), neither)],
        ..Default::default()
    };
    assert!(
        upcoming_dividends(&query, &shares(&[("2330", 1)]), day(19), day(19))
            .await
            .is_empty()
    );
}

/// 現金入帳 = 股數 × 每股現金股利；配股以面額 10 元換算並捨去不足一股。
#[test]
fn amounts_follow_shares() {
    let item = UpcomingDividend {
        ex_date: day(19),
        stock_symbol: "2412".to_owned(),
        name: String::new(),
        cash_dividend: dec!(1.5),
        stock_dividend: dec!(0.55),
        closing_price: dec!(100),
        shares: 1999,
    };
    assert_eq!(item.cash_amount(), dec!(2998.5));
    assert_eq!(
        item.stock_shares(),
        dec!(109),
        "1999 × 0.55 ÷ 10 = 109.945 捨去"
    );
}
