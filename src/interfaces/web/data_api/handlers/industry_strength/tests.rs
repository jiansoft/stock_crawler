//! `/market/industry-strength` handler：各產業成分股相對強度的分布，用來看資金輪動到哪些產業的測試。

use super::super::relative_strength::UniverseRow;
use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

fn stock(symbol: &str, market_id: i32, industry: &str, rs_score: u8) -> ScoredStock {
    ScoredStock {
        rank: 1,
        rs_score,
        weighted: f64::from(rs_score) / 10.0,
        row: UniverseRow {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol}名稱"),
            market_id,
            industry: industry.to_owned(),
            return_20d: None,
            return_60d: 0.0,
            return_120d: 0.0,
            return_240d: 0.0,
            closing_price: None,
        },
    }
}

/// 由強到弱的全體：半導體 4 檔、航運 3 檔、觀光 1 檔、未分類 1 檔。
fn universe() -> Vec<ScoredStock> {
    vec![
        stock("A", 2, "半導體業", 99),
        stock("B", 4, "半導體業", 90),
        stock("C", 2, "航運業", 85),
        stock("D", 2, "半導體業", 70),
        stock("E", 2, "觀光業", 60),
        stock("F", 4, "航運業", 50),
        stock("G", 2, "", 40),
        stock("H", 2, "半導體業", 20),
        stock("I", 2, "航運業", 15),
    ]
}

/// 中位數（奇偶數檔）、強弱勢占比、平均加權報酬、領先股、前期比較與檔數門檻。
#[test]
fn build_response_aggregates_industries() {
    let prior = vec![
        stock("A", 2, "半導體業", 60),
        stock("B", 4, "半導體業", 40),
        stock("C", 2, "航運業", 30),
    ];
    let response = build_response(
        day(2026, 10, 13),
        "all",
        0,
        3,
        "desc",
        universe(),
        Some((day(2026, 9, 14), prior)),
    );
    assert_eq!(response.data_as_of, "2026-10-13");
    assert_eq!(response.prior_as_of.as_deref(), Some("2026-09-14"));
    assert_eq!(response.universe, 9);
    let names: Vec<&str> = response.items.iter().map(|i| i.industry.as_str()).collect();
    assert_eq!(names, ["半導體業", "航運業"], "觀光業只有 1 檔、未分類略過");

    let semi = &response.items[0];
    assert_eq!(semi.rank, 1);
    assert_eq!(semi.stocks, 4);
    // 20、70、90、99 → (70 + 90) ÷ 2。
    assert_eq!(semi.median_rs_score, 80.0);
    assert_eq!(semi.strong_ratio, 0.5);
    assert_eq!(semi.weak_ratio, 0.25);
    // (9.9 + 9.0 + 7.0 + 2.0) ÷ 4 = 6.975 → 6.98。
    assert_eq!(semi.avg_weighted_return, 6.98);
    assert_eq!(semi.prior_median_rs_score, Some(50.0));
    assert_eq!(semi.median_change, Some(30.0));
    let leaders: Vec<(&str, u8)> = semi
        .leaders
        .iter()
        .map(|l| (l.stock_symbol.as_str(), l.rs_score))
        .collect();
    assert_eq!(leaders, [("A", 99), ("B", 90), ("D", 70)]);
    assert_eq!(semi.leaders[0].name, "A名稱");

    let shipping = &response.items[1];
    assert_eq!((shipping.rank, shipping.median_rs_score), (2, 50.0));
    assert_eq!(shipping.weak_ratio, 0.3333);
    assert_eq!(shipping.prior_median_rs_score, Some(30.0));

    let otc_asc = build_response(day(2026, 10, 13), "tpex", 4, 1, "asc", universe(), None);
    assert!(otc_asc.prior_as_of.is_none());
    assert_eq!(otc_asc.order, "asc");
    let otc: Vec<(&str, usize, f64)> = otc_asc
        .items
        .iter()
        .map(|i| (i.industry.as_str(), i.rank, i.median_rs_score))
        .collect();
    assert_eq!(otc, [("航運業", 2, 50.0), ("半導體業", 1, 90.0)]);
    assert!(otc_asc.items[0].median_change.is_none());
}

/// 同中位數同名次，再依產業名稱排列。
#[test]
fn build_response_ties_share_rank() {
    let scored = vec![
        stock("A", 2, "乙", 60),
        stock("B", 2, "甲", 60),
        stock("C", 2, "丙", 30),
    ];
    let response = build_response(day(2026, 10, 13), "all", 0, 1, "desc", scored, None);
    let ranked: Vec<(&str, usize)> = response
        .items
        .iter()
        .map(|i| (i.industry.as_str(), i.rank))
        .collect();
    assert_eq!(ranked, [("乙", 1), ("甲", 1), ("丙", 3)]);
    assert_eq!(median_score(&[]), None);
}
