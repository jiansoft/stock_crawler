//! 下週持股除權息一段：期間計算、只留持股、單日失敗略過、排序與合計、沒有事件不列、run 整合。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::super::upcoming_dividend::{
    ExDividendDayQuery, UpcomingDividend, UpcomingSection, lines, next_week, total_shares,
    upcoming_events, upcoming_section,
};
use super::fakes::*;
use super::*;
use crate::app::calculation::dividend_fill_hints::FillHints;
use crate::domain::dividend::entity::StockDividendInfo;
use crate::domain::dividend::fill::FillSummary;

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

/// 週六、週日、週一執行都取「接下來的」週一到週五。
#[test]
fn next_week_starts_on_the_coming_monday() {
    assert_eq!(next_week(day(17)), (day(19), day(23)), "週六");
    assert_eq!(next_week(day(18)), (day(19), day(23)), "週日");
    assert_eq!(next_week(day(19)), (day(26), day(30)), "週一");
}

/// 同一代號分次買進、不同會員的股數合併。
#[test]
fn total_shares_merges_lots() {
    assert_eq!(
        shares(&[("2330", 100), ("0050", 50), ("2330", 400)]),
        BTreeMap::from([("0050".to_owned(), 50), ("2330".to_owned(), 500)])
    );
}

/// 查下週一到週五共 5 天，只留持股；單日失敗只少那天；沒有持股不查。
#[tokio::test]
async fn upcoming_events_filters_holdings_and_skips_failed_days() {
    let query = FakeDays {
        by_day: vec![
            (day(19), info("2330", dec!(5), Decimal::ZERO)),
            (day(19), info("1101", dec!(1), Decimal::ZERO)),
            (day(21), info("0056", dec!(1), Decimal::ZERO)),
            (day(26), info("2330", dec!(5), Decimal::ZERO)),
        ],
        fail_on: Some(day(21)),
        ..Default::default()
    };
    let held = BTreeSet::from(["2330".to_owned(), "0056".to_owned()]);
    let events = upcoming_events(&query, &held, day(17)).await;
    assert_eq!(
        events
            .iter()
            .map(|(d, stock)| (*d, stock.stock_symbol.as_str()))
            .collect::<Vec<_>>(),
        vec![(day(19), "2330")]
    );
    assert_eq!(
        *query.asked.lock().expect("未中毒"),
        vec![day(19), day(20), day(21), day(22), day(23)]
    );

    let empty = FakeDays::default();
    assert!(
        upcoming_events(&empty, &BTreeSet::new(), day(17))
            .await
            .is_empty()
    );
    assert!(
        empty.asked.lock().expect("未中毒").is_empty(),
        "沒有持股不查"
    );
}

/// 依除權息日、代號排序；只取這天的除息或除權；附填息文字；沒有事件回 None。
#[test]
fn upcoming_section_sorts_and_attaches_fill_hints() {
    let hints = FillHints::for_test(
        HashMap::from([(
            "2330".to_owned(),
            FillSummary {
                events: 5,
                filled: 4,
                average_trading_days: Some(3.0),
                median_trading_days: Some(2.0),
            },
        )]),
        Vec::new(),
    );
    let mut only_stock_today = info("2412", dec!(4), dec!(1));
    only_stock_today.is_cash_ex_dividend_on_date = false;
    let events = vec![
        (day(21), info("2330", dec!(5), Decimal::ZERO)),
        (day(19), only_stock_today),
        (day(19), info("9999", dec!(1), Decimal::ZERO)),
    ];
    let section = upcoming_section(
        events,
        &shares(&[("2330", 1000), ("2412", 2000)]),
        &hints,
        day(17),
    )
    .expect("有持股除權息");
    assert_eq!((section.from, section.to), (day(19), day(23)));
    assert_eq!(
        section.items,
        vec![
            UpcomingDividend {
                ex_date: day(19),
                stock_symbol: "2412".to_owned(),
                name: "名2412".to_owned(),
                cash_dividend: Decimal::ZERO,
                stock_dividend: dec!(1),
                shares: 2000,
                fill: None,
            },
            UpcomingDividend {
                ex_date: day(21),
                stock_symbol: "2330".to_owned(),
                name: "名2330".to_owned(),
                cash_dividend: dec!(5),
                stock_dividend: Decimal::ZERO,
                shares: 1000,
                fill: Some("近 5 年填息 4／5 次、平均 3 日".to_owned()),
            },
        ]
    );

    assert!(upcoming_section(Vec::new(), &shares(&[]), &hints, day(17)).is_none());
    let mut neither = info("2330", dec!(5), Decimal::ZERO);
    neither.is_cash_ex_dividend_on_date = false;
    assert!(
        upcoming_section(
            vec![(day(19), neither)],
            &shares(&[("2330", 1)]),
            &hints,
            day(17)
        )
        .is_none(),
        "這天既不除息也不除權不列"
    );
}

/// 每檔一行：日期、代號、股數、除息與除權、填息；兩檔以上才附現金合計。
#[test]
fn lines_show_amounts_and_total() {
    let item = |symbol: &str, cash: Decimal, stock: Decimal, fill: Option<&str>| UpcomingDividend {
        ex_date: day(20),
        stock_symbol: symbol.to_owned(),
        name: format!("名{symbol}"),
        cash_dividend: cash,
        stock_dividend: stock,
        shares: 1500,
        fill: fill.map(str::to_owned),
    };
    let section = UpcomingSection {
        from: day(19),
        to: day(23),
        items: vec![
            item("2330", dec!(5), Decimal::ZERO, Some("近 5 年填息 4／5 次")),
            item("2412", dec!(1.5), dec!(0.5), None),
        ],
    };
    assert_eq!(
        lines(&section),
        vec![
            String::new(),
            "下週持股除權息（10-19～10-23）".to_owned(),
            "• 10-20 2330 名2330（1,500 股）除息 5 元、預估入帳 7,500 元；近 5 年填息 4／5 次"
                .to_owned(),
            "• 10-20 2412 名2412（1,500 股）除息 1.5 元、預估入帳 2,250 元；除權 0.5 元、預估配股 75 股"
                .to_owned(),
            "• 合計預估現金入帳 9,750 元（未扣二代健保與稅）".to_owned(),
        ]
    );
    let single = UpcomingSection {
        items: vec![item("2330", dec!(5), Decimal::ZERO, None)],
        ..section
    };
    assert_eq!(lines(&single).len(), 3, "只有一檔不附合計");
}

/// run 把下週除權息接在持股填息之後。
#[tokio::test]
async fn run_includes_upcoming_dividends() {
    let section = UpcomingSection {
        from: day(19),
        to: day(23),
        items: vec![UpcomingDividend {
            ex_date: day(20),
            stock_symbol: "2330".to_owned(),
            name: "台積電".to_owned(),
            cash_dividend: dec!(5),
            stock_dividend: Decimal::ZERO,
            shares: 1000,
            fill: None,
        }],
    };
    let message = run(
        &quote(),
        &chip(),
        &index(),
        &FakeValuation::default(),
        &portfolio(),
        day(17),
        &holdings(),
        None,
        &[],
        None,
        &[],
        Some(&section),
    )
    .await
    .expect("有內容");
    assert!(
        message.contains(&esc(
            "• 10-20 2330 台積電（1,000 股）除息 5 元、預估入帳 5,000 元"
        )),
        "{message}"
    );
}
