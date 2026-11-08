//! `/stocks/{symbol}/relative-strength` handler：個股近 N 個交易日的相對強度走勢的測試。

use super::*;

fn row(day: u32, weaker: i64, universe: i64) -> RelativeStrengthDay {
    RelativeStrengthDay {
        stock_symbol: "2330".to_owned(),
        date: chrono::NaiveDate::from_ymd_opt(2026, 10, day).expect("合法日期"),
        weighted: 12.345,
        rs_score: crate::domain::quote::relative_strength::percentile_score(
            usize::try_from(weaker).expect("測試資料非負"),
            usize::try_from(universe).expect("測試資料非負"),
        ),
        rank: universe - weaker,
        universe,
    }
}

/// 分數取自每日分數；高低點同分取最近一次；變化＝最新減最舊；穿越由新到舊。
#[test]
fn build_history_scores_extremes_and_crossings() {
    // 由新到舊：13 日 99、12 日 50、9 日 40、8 日 85、7 日 99（universe 101 → 分數＝weaker × 98 ÷ 100 + 1）。
    let rows = vec![
        row(13, 100, 101),
        row(12, 50, 101),
        row(9, 40, 101),
        row(8, 86, 101),
        row(7, 100, 101),
    ];
    let response = build_history("2330".to_owned(), 60, rows);
    let scores: Vec<u8> = response.items.iter().map(|item| item.rs_score).collect();
    assert_eq!(scores, [99, 50, 40, 85, 99]);
    assert_eq!(response.data_as_of.as_deref(), Some("2026-10-13"));
    assert_eq!(response.latest_score, Some(99));
    assert_eq!(
        (response.high_score, response.high_date.as_deref()),
        (Some(99), Some("2026-10-13")),
        "同分取最近一次"
    );
    assert_eq!(
        (response.low_score, response.low_date.as_deref()),
        (Some(40), Some("2026-10-09"))
    );
    assert_eq!(response.score_change, Some(0));
    assert_eq!(response.items[0].rank, 1);
    assert_eq!(response.items[0].universe, 101);
    assert_eq!(response.items[0].weighted_return, 12.35);

    let crossings: Vec<(&str, u8, &str, u8, u8)> = response
        .crossings
        .iter()
        .map(|c| {
            (
                c.date.as_str(),
                c.threshold,
                c.direction.as_str(),
                c.from_score,
                c.to_score,
            )
        })
        .collect();
    assert_eq!(
        crossings,
        [
            ("2026-10-13", 80, "up", 50, 99),
            ("2026-10-12", 50, "up", 40, 50),
            ("2026-10-09", 80, "down", 85, 40),
            ("2026-10-09", 50, "down", 85, 40),
        ]
    );
}

/// 期間內沒有分數：所有摘要欄位為 null、清單為空。
#[test]
fn build_history_empty() {
    let response = build_history("7799".to_owned(), 20, Vec::new());
    assert_eq!(response.days, 20);
    assert!(response.data_as_of.is_none());
    assert!(response.latest_score.is_none());
    assert!(response.high_date.is_none());
    assert!(response.low_score.is_none());
    assert!(response.score_change.is_none());
    assert!(response.crossings.is_empty());
    assert!(response.items.is_empty());
}
