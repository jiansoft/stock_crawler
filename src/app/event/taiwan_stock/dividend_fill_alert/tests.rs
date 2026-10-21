//! 持股填息通知的測試：只通知今天填息、期間過濾、單檔失敗略過與訊息格式。

use anyhow::anyhow;
use async_trait::async_trait;
use rust_decimal_macros::dec;

use super::*;
use crate::domain::dividend::fill::{DailyClose, ExRightEvent};

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("日期應合法")
}

fn event(ex_date: NaiveDate, cash: Decimal, stock: Decimal) -> ExRightEvent {
    ExRightEvent {
        ex_date,
        cash_dividend: cash,
        stock_dividend: stock,
    }
}

/// 2330 10-14 除息、前一日收 100，10-21 才回到 100（今天填息）；1101 10-15 除息、10-16 就填息；
/// 0056 去年 9 月除息（超過一年）不看；9999 查詢失敗。
struct FakeFills;

#[async_trait]
impl DividendFillQuery for FakeFills {
    async fn fetch_latest_quote_date(&self, _symbol: &str) -> Result<Option<NaiveDate>> {
        unreachable!("填息通知不需要最新收盤日")
    }

    async fn fetch_ex_right_events(&self, symbol: &str) -> Result<Vec<ExRightEvent>> {
        match symbol {
            "2330" => Ok(vec![
                event(day(28), dec!(5), Decimal::ZERO),
                event(day(14), dec!(5), dec!(0.5)),
            ]),
            "1101" => Ok(vec![event(day(15), dec!(1), Decimal::ZERO)]),
            "0056" => Ok(vec![event(
                NaiveDate::from_ymd_opt(2025, 9, 1).expect("日期應合法"),
                dec!(1),
                Decimal::ZERO,
            )]),
            "9999" => Err(anyhow!("db down")),
            _ => Ok(Vec::new()),
        }
    }

    async fn fetch_daily_closes(&self, symbol: &str, since: NaiveDate) -> Result<Vec<DailyClose>> {
        let closes: &[(u32, Decimal)] = match symbol {
            "2330" => {
                assert_eq!(since, day(14) - Days::new(PRE_EX_LOOKBACK_DAYS));
                &[
                    (13, dec!(100)),
                    (14, dec!(95)),
                    (15, dec!(96)),
                    (16, dec!(99.5)),
                    (20, dec!(98)),
                    (21, dec!(100)),
                ]
            }
            "1101" => &[
                (14, dec!(30)),
                (15, dec!(29)),
                (16, dec!(30)),
                (21, dec!(31)),
            ],
            _ => panic!("不應查 {symbol} 的日線"),
        };
        Ok(closes
            .iter()
            .map(|&(d, close)| DailyClose {
                date: day(d),
                close,
            })
            .collect())
    }
}

/// 只留填息日是今天的；未來的除息日、超過一年的與查詢失敗的略過。
#[tokio::test]
async fn filled_today_keeps_only_todays_fills() {
    let symbols: Vec<String> = ["0056", "1101", "2330", "2454", "9999"]
        .map(str::to_owned)
        .to_vec();
    let filled = filled_today(&FakeFills, &symbols, day(21)).await;
    assert_eq!(filled.len(), 1);
    let (symbol, outcome) = &filled[0];
    assert_eq!(symbol, "2330");
    assert_eq!(outcome.event.ex_date, day(14));
    assert_eq!(outcome.trading_days_to_fill, Some(5));
    assert_eq!(outcome.pre_ex_close, dec!(100));

    assert!(
        filled_today(&FakeFills, &symbols, day(20)).await.is_empty(),
        "10-20 收 98 還沒填息，1101 10-16 已填息不是今天"
    );
}

/// 訊息列出除權息日、股利、前一日收盤與今日收盤、第幾個交易日填息；沒有填息時不發送。
#[tokio::test]
async fn build_message_lists_each_fill() {
    let symbols = vec!["2330".to_owned()];
    let filled = filled_today(&FakeFills, &symbols, day(21)).await;
    let message = build_message(day(21), &filled, |_| "台積電".to_owned()).expect("有填息");
    assert!(message.starts_with(&text::escape_markdown_v2("持股填息︰2026-10-21\n")));
    let line = "2330 台積電 除權息 2026-10-14（現金 5 元、股票 0.5 元）：前一日收盤 100，今日收盤 100，第 5 個交易日填息";
    assert!(
        message.contains(&text::escape_markdown_v2(line)),
        "{message}"
    );
    assert!(build_message(day(21), &[], |_| String::new()).is_none());
    assert_eq!(dividend_text(dec!(1.50), Decimal::ZERO), "現金 1.5 元");
}
