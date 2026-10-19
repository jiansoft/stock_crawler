//! 個股事件時間軸的單元測試。

use rust_decimal_macros::dec;

use super::*;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("固定日期合法")
}

fn row(kind: &str) -> TimelineRow {
    TimelineRow {
        date: day(10, 1),
        kind: kind.to_owned(),
        ..Default::default()
    }
}

/// 逗號分隔、去掉空白與重複；省略或空字串為全部；不認得的種類回錯誤。
#[test]
fn parse_kinds_splits_and_validates() {
    assert_eq!(parse_kinds(None), Ok(None));
    assert_eq!(parse_kinds(Some(" , ")), Ok(None));
    assert_eq!(
        parse_kinds(Some("signal, ex_dividend,signal")),
        Ok(Some(vec!["signal".to_owned(), "ex_dividend".to_owned()]))
    );
    assert!(parse_kinds(Some("signal,earnings_call")).is_err());
}

/// 各種類的標題與說明：數字去掉多餘的 0、百分比帶正負號、張數四捨五入。
#[test]
fn describe_formats_each_kind() {
    let cases = [
        (
            TimelineRow {
                v1: Some(dec!(4.50)),
                period: Some("2026Q2".to_owned()),
                ..row("ex_dividend")
            },
            ("除息：現金股利 4.5 元", Some("2026 年 Q2 股利")),
        ),
        (
            TimelineRow {
                v1: Some(dec!(1)),
                period: Some("2025".to_owned()),
                ..row("ex_right")
            },
            ("除權：股票股利 1 元", Some("2025 年度股利")),
        ),
        (
            TimelineRow {
                v1: Some(dec!(4.5)),
                ..row("cash_payable")
            },
            ("現金股利發放 4.5 元", None),
        ),
        (
            TimelineRow {
                v1: Some(dec!(0.5)),
                ..row("stock_payable")
            },
            ("股票股利發放 0.5 元", None),
        ),
        (
            TimelineRow {
                label: Some("capital_reduction".to_owned()),
                note: Some("減資彌補虧損".to_owned()),
                v1: Some(dec!(0.83068288)),
                ..row("corporate_action")
            },
            ("減資生效（換發比例 0.83）", Some("減資彌補虧損")),
        ),
        (
            TimelineRow {
                label: Some("split".to_owned()),
                note: Some(String::new()),
                v1: Some(dec!(4)),
                ..row("corporate_action")
            },
            ("分割生效（換發比例 4）", None),
        ),
        (
            TimelineRow {
                period: Some("2026-09".to_owned()),
                v1: Some(dec!(54.64)),
                v2: Some(dec!(-0.57)),
                v3: Some(dec!(511857399)),
                ..row("monthly_revenue")
            },
            (
                "2026-09 營收年增 +54.64%",
                Some("營收 5118.57 億元、月增 -0.57%"),
            ),
        ),
        (
            TimelineRow {
                period: Some("2026-09".to_owned()),
                ..row("monthly_revenue")
            },
            ("2026-09 營收公布", None),
        ),
        (
            TimelineRow {
                period: Some("2026Q2".to_owned()),
                v1: Some(dec!(15.36)),
                v2: Some(dec!(8.20)),
                ..row("quarterly_report")
            },
            ("2026Q2 財報：EPS 15.36 元", Some("ROE 8.2%")),
        ),
        (
            TimelineRow {
                label: Some("kd".to_owned()),
                note: Some("KD 黃金交叉".to_owned()),
                ..row("signal")
            },
            ("KD 黃金交叉", None),
        ),
        (
            TimelineRow {
                label: Some("foreign".to_owned()),
                v1: Some(dec!(10)),
                v2: Some(dec!(1234567)),
                ..row("chip_streak")
            },
            ("外資連買 10 天", Some("當天買賣超 +1235 張")),
        ),
        (
            TimelineRow {
                label: Some("trust".to_owned()),
                v1: Some(dec!(-5)),
                v2: Some(dec!(-2400)),
                ..row("chip_streak")
            },
            ("投信連賣 5 天", Some("當天買賣超 -2 張")),
        ),
        (
            TimelineRow {
                period: Some("2026-09".to_owned()),
                v1: Some(dec!(-1.234)),
                v2: Some(dec!(12.5)),
                v3: Some(dec!(1.5)),
                ..row("insider")
            },
            (
                "2026-09 董監持股 -1.23%",
                Some("質押比例 12.5%（+1.5 個百分點）"),
            ),
        ),
        (row("unknown"), ("unknown", None)),
    ];
    for (row, (title, detail)) in cases {
        assert_eq!(
            describe(&row),
            (title.to_owned(), detail.map(str::to_owned)),
            "{row:?}"
        );
    }
}

/// 事件由新到舊、同一天依種類順序；迄日之後標記 upcoming；筆數依種類順序；沒有事件為空。
#[test]
fn build_timeline_sorts_and_flags_upcoming() {
    let counts = |row: TimelineRow| TimelineRow {
        total: 3,
        count_kinds: vec!["ex_dividend".to_owned(), "signal".to_owned()],
        count_values: vec![1, 2],
        ..row
    };
    let rows = vec![
        counts(TimelineRow {
            date: day(10, 1),
            note: Some("KD 黃金交叉".to_owned()),
            ..row("signal")
        }),
        counts(TimelineRow {
            date: day(10, 1),
            v1: Some(dec!(1)),
            ..row("ex_dividend")
        }),
        counts(TimelineRow {
            date: day(11, 5),
            note: Some("MACD 黃金交叉".to_owned()),
            ..row("signal")
        }),
    ];
    let response = build_timeline(
        "2330".to_owned(),
        "台積電".to_owned(),
        day(8, 1),
        day(10, 20),
        day(12, 19),
        rows,
    );
    assert_eq!(response.total, 3);
    assert_eq!(response.upcoming_to, "2026-12-19");
    assert_eq!(
        response
            .counts
            .iter()
            .map(|count| (count.kind.as_str(), count.count))
            .collect::<Vec<_>>(),
        [("ex_dividend", 1), ("signal", 2)]
    );
    assert_eq!(
        response
            .events
            .iter()
            .map(|event| (event.date.as_str(), event.kind.as_str(), event.upcoming))
            .collect::<Vec<_>>(),
        [
            ("2026-11-05", "signal", true),
            ("2026-10-01", "ex_dividend", false),
            ("2026-10-01", "signal", false),
        ]
    );

    let empty = build_timeline(
        "2330".to_owned(),
        "台積電".to_owned(),
        day(8, 1),
        day(10, 20),
        day(10, 20),
        Vec::new(),
    );
    assert_eq!(empty.total, 0);
    assert!(empty.counts.is_empty() && empty.events.is_empty());
}
