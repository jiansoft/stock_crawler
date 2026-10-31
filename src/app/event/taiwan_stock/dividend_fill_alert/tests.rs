//! 持股填息通知的測試：今天填息、今天第一次落後基準、期間過濾、單檔失敗略過與訊息格式。

use anyhow::anyhow;
use async_trait::async_trait;
use rust_decimal_macros::dec;

use super::*;
use crate::domain::dividend::fill::{DailyClose, ExRightEvent};
use crate::domain::dividend::fill_stats::FillSample;

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

fn y2025(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, m, d).expect("日期應合法")
}

/// 2330 10-14 除權息（配股 0.5 元）、前一日收 105，填息目標 105 ÷ 1.05 = 100，10-21 才回到 100（今天填息）；
/// 1101 10-15 除息、10-16 就填息；9999 查詢失敗。
/// 0056 去年 9 月除息第 2 天填息（超過一年不通知，但算進近 5 年平均 2 日），10-15 再除息後一直沒填息，
/// 10-20 已過 3 個交易日＝第一次超過平均；2884 10-15 除息（殖利率 5%）沒有自身紀錄，
/// 10-21 已過 4 個交易日＝第一次超過同組中位數 3 日。
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
            "0056" => Ok(vec![
                event(day(15), dec!(1), Decimal::ZERO),
                event(y2025(9, 1), dec!(1), Decimal::ZERO),
            ]),
            "2884" => Ok(vec![event(day(15), dec!(1), Decimal::ZERO)]),
            "9999" => Err(anyhow!("db down")),
            _ => Ok(Vec::new()),
        }
    }

    async fn fetch_daily_closes(&self, symbol: &str, since: NaiveDate) -> Result<Vec<DailyClose>> {
        if symbol == "0056" {
            assert_eq!(since, y2025(9, 1) - Days::new(fill::PRE_EX_LOOKBACK_DAYS));
            let closes = [
                (y2025(8, 29), dec!(40)),
                (y2025(9, 1), dec!(38)),
                (y2025(9, 2), dec!(40.5)),
                (day(14), dec!(36)),
                (day(15), dec!(35)),
                (day(16), dec!(35)),
                (day(20), dec!(35)),
            ];
            return Ok(closes
                .into_iter()
                .map(|(date, close)| DailyClose { date, close })
                .collect());
        }
        let closes: &[(u32, Decimal)] = match symbol {
            "2330" => {
                assert_eq!(since, day(14) - Days::new(fill::PRE_EX_LOOKBACK_DAYS));
                &[
                    (13, dec!(105)),
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
            "2884" => &[
                (14, dec!(20)),
                (15, dec!(19)),
                (16, dec!(19)),
                (20, dec!(19)),
                (21, dec!(19)),
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

const SYMBOLS: [&str; 6] = ["0056", "1101", "2330", "2454", "2884", "9999"];

fn symbols() -> Vec<String> {
    SYMBOLS.map(str::to_owned).to_vec()
}

/// 全市場基準：只有 5～7% 組有中位數 3 日。
fn peers() -> Vec<BucketStats> {
    bucket_stats(&[FillSample {
        bucket: YieldBucket::From5To7,
        trading_days_to_fill: Some(3),
        trading_days_observed: 120,
    }])
}

/// 今天填息的只留填息日是今天的；未來的除息日、超過一年的與查詢失敗的略過。
/// 落後只在已過天數第一次超過基準那天出現：0056 10-20（自身平均 2 日）、2884 10-21（同組中位數 3 日）。
#[tokio::test]
async fn review_finds_todays_fills_and_first_lags() {
    let today = review(&FakeFills, &symbols(), &peers(), day(21)).await;
    assert_eq!(today.filled.len(), 1);
    let (symbol, outcome) = &today.filled[0];
    assert_eq!(symbol, "2330");
    assert_eq!(outcome.event.ex_date, day(14));
    assert_eq!(outcome.trading_days_to_fill, Some(5));
    assert_eq!(
        (outcome.pre_ex_close, outcome.fill_target),
        (dec!(105), dec!(100))
    );
    let lagging: Vec<(&str, u32, Baseline)> = today
        .lagging
        .iter()
        .map(|l| {
            (
                l.symbol.as_str(),
                l.outcome.trading_days_since_ex,
                l.baseline,
            )
        })
        .collect();
    assert_eq!(
        lagging,
        [(
            "2884",
            4,
            Baseline::Peer {
                bucket: YieldBucket::From5To7,
                median: 3.0
            }
        )],
        "0056 10-21 沒有收盤（不是今天的資料），2330 的 4.76% 組沒有基準"
    );

    let yesterday = review(&FakeFills, &symbols(), &peers(), day(20)).await;
    assert!(
        yesterday.filled.is_empty(),
        "10-20 收 98 還沒填息，1101 10-16 已填息不是今天"
    );
    let lagging: Vec<(&str, Baseline)> = yesterday
        .lagging
        .iter()
        .map(|l| (l.symbol.as_str(), l.baseline))
        .collect();
    assert_eq!(
        lagging,
        [("0056", Baseline::Own(2.0))],
        "2884 才過 3 天、沒超過 3 日"
    );

    let no_peers = review(&FakeFills, &symbols(), &[], day(21)).await;
    assert!(no_peers.lagging.is_empty(), "沒有全市場基準時只用自身紀錄");
}

/// 基準取整數加 1 那天才成立；已填息、最新收盤不是今天、沒有基準時都不判斷。
#[test]
fn lag_baseline_fires_only_on_the_first_exceeding_day() {
    let outcome = |since_ex: u32, filled: bool, latest: u32| FillOutcome {
        event: event(day(1), dec!(1), Decimal::ZERO),
        pre_ex_date: NaiveDate::from_ymd_opt(2026, 9, 30).expect("日期應合法"),
        pre_ex_close: dec!(20),
        fill_target: dec!(20),
        fill_date: filled.then(|| day(2)),
        trading_days_to_fill: filled.then_some(2),
        trading_days_since_ex: since_ex,
        latest_close: dec!(19),
        latest_date: day(latest),
    };
    let own = FillSummary {
        events: 3,
        filled: 2,
        average_trading_days: Some(12.5),
        median_trading_days: Some(12.5),
    };
    let none = FillSummary {
        events: 1,
        filled: 0,
        average_trading_days: None,
        median_trading_days: None,
    };
    let today = day(21);
    assert_eq!(
        lag_baseline(&outcome(13, false, 21), &own, &[], today),
        Some(Baseline::Own(12.5))
    );
    for since_ex in [12, 14] {
        assert_eq!(
            lag_baseline(&outcome(since_ex, false, 21), &own, &[], today),
            None
        );
    }
    assert_eq!(lag_baseline(&outcome(13, true, 21), &own, &[], today), None);
    assert_eq!(
        lag_baseline(&outcome(13, false, 20), &own, &[], today),
        None
    );
    assert_eq!(
        lag_baseline(&outcome(4, false, 21), &none, &peers(), today),
        Some(Baseline::Peer {
            bucket: YieldBucket::From5To7,
            median: 3.0
        }),
        "現金 1 元 ÷ 20 = 5%"
    );
    assert_eq!(
        lag_baseline(&outcome(4, false, 21), &none, &[], today),
        None
    );
}

/// 訊息列出今天填息的（有配股時註明前一日收盤）與今天第一次落後的（附基準與附註）；兩者都沒有時不發送。
#[tokio::test]
async fn build_message_lists_fills_and_lags() {
    let today = review(&FakeFills, &symbols(), &peers(), day(21)).await;
    let name = |symbol: &str| match symbol {
        "2330" => "台積電".to_owned(),
        _ => "兆豐金".to_owned(),
    };
    let message = build_message(day(21), &today, name).expect("有填息");
    assert!(message.starts_with(&text::escape_markdown_v2("持股填息︰2026-10-21\n")));
    for line in [
        "2330 台積電 除權息 2026-10-14（現金 5 元、股票 0.5 元）：填息目標 100（前一日收盤 105 依配股調整），今日收盤 100，第 5 個交易日填息",
        "填息落後（今天第一次超過基準）：",
        "2884 兆豐金 除權息 2026-10-15（現金 1 元）：已 4 個交易日未填息，超過同殖利率組（5～7%）全市場中位數 3 日；最新收盤 19，距填息價 20 還差 5%",
        LAG_FOOTNOTE,
    ] {
        assert!(
            message.contains(&text::escape_markdown_v2(line)),
            "缺少 {line}：{message}"
        );
    }

    let yesterday = review(&FakeFills, &symbols(), &peers(), day(20)).await;
    let message = build_message(day(20), &yesterday, |_| "元大高股息".to_owned()).expect("有落後");
    assert!(
        message.contains(&text::escape_markdown_v2("超過近 5 年平均 2 日")),
        "{message}"
    );

    assert!(build_message(day(21), &Review::default(), |_| String::new()).is_none());
    assert_eq!(dividend_text(dec!(1.50), Decimal::ZERO), "現金 1.5 元");
}
