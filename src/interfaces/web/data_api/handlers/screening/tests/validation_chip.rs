//! 條件選股的相對強度、法人連買與千張大戶參數驗證。

use super::*;

/// 相對強度門檻 1–99 含兩端，本身就是有效篩選條件；轉成 SQL 的 smallint。
#[test]
fn screening_min_rs_score_boundaries() {
    for (score, ok) in [(0, false), (1, true), (99, true), (100, false)] {
        let mut params = screening_params();
        params.min_rs_score = Some(score);
        let result = validate_screening_params(&params);
        assert_eq!(result.is_ok(), ok, "{score}");
        if let Ok(validated) = result {
            assert_eq!(validated.min_rs_score, Some(i16::from(score)));
        } else {
            assert_eq!(result.err(), Some("min_rs_score 必須介於 1 至 99"));
        }
    }
    let mut sorted = screening_params();
    sorted.market = Some("tpex".to_owned());
    sorted.sort_by = Some("rs_score".to_owned());
    sorted.sort_order = Some("desc".to_owned());
    let validated = validate_screening_params(&sorted).expect("rs_score 可排序");
    assert_eq!(
        validated.order_by,
        "ORDER BY rs_score DESC NULLS LAST, stock_symbol ASC"
    );
    assert_eq!(validated.min_rs_score, None);
}

/// 全體計分拆成代號、分數兩個平行陣列；查無相對強度資料時資料日為 None、陣列為空。
#[test]
fn screening_rs_columns_split_scores() {
    let scored = |symbol: &str, rs_score: u8| ScoredStock {
        rank: 1,
        rs_score,
        weighted: 1.0,
        row: UniverseRow {
            stock_symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            market_id: 2,
            industry: String::new(),
            return_20d: None,
            return_60d: 1.0,
            return_120d: 1.0,
            return_240d: 1.0,
            closing_price: None,
        },
    };
    let date = NaiveDate::from_ymd_opt(2026, 10, 13).expect("合法日期");
    let (as_of, symbols, scores) =
        rs_columns(Some((date, vec![scored("2330", 99), scored("2317", 42)])));
    assert_eq!(as_of.as_deref(), Some("2026-10-13"));
    assert_eq!(symbols, ["2330", "2317"]);
    assert_eq!(scores, [99, 42]);
    assert_eq!(rs_columns(None), (None, Vec::new(), Vec::new()));
}

/// 外資／投信連續買超門檻 1–60 含兩端，本身就是有效篩選條件。
#[test]
fn screening_min_streak_boundaries() {
    for (days, ok) in [(0, false), (1, true), (60, true), (61, false)] {
        let mut foreign = screening_params();
        foreign.min_foreign_streak = Some(days);
        let result = validate_screening_params(&foreign);
        assert_eq!(result.is_ok(), ok, "foreign {days}");
        match result {
            Ok(validated) => {
                assert_eq!(validated.min_foreign_streak, Some(i16::from(days)));
                assert_eq!(validated.min_trust_streak, None);
            }
            Err(message) => assert_eq!(message, "min_foreign_streak 必須介於 1 至 60"),
        }

        let mut trust = screening_params();
        trust.min_trust_streak = Some(days);
        let result = validate_screening_params(&trust);
        assert_eq!(result.is_ok(), ok, "trust {days}");
        match result {
            Ok(validated) => assert_eq!(validated.min_trust_streak, Some(i16::from(days))),
            Err(message) => assert_eq!(message, "min_trust_streak 必須介於 1 至 60"),
        }
    }
}

/// 連續天數拆成代號、外資、投信三個平行陣列；查無籌碼資料時資料日為 None、陣列為空。
#[test]
fn screening_streak_columns_split_rows() {
    let streak = |symbol: &str, foreign_streak: i16, trust_streak: i16| StreakRow {
        stock_symbol: symbol.to_owned(),
        foreign_streak,
        trust_streak,
    };
    let date = NaiveDate::from_ymd_opt(2026, 10, 13).expect("合法日期");
    let (as_of, symbols, foreign, trust) = streak_columns(Some((
        date,
        vec![streak("2330", -5, 10), streak("6505", 0, 14)],
    )));
    assert_eq!(as_of.as_deref(), Some("2026-10-13"));
    assert_eq!(symbols, ["2330", "6505"]);
    assert_eq!(foreign, [-5, 0]);
    assert_eq!(trust, [10, 14]);
    assert_eq!(
        streak_columns(None),
        (None, Vec::new(), Vec::new(), Vec::new())
    );
}

/// 千張大戶變化：下限 -100～100（可為負）、回看週數 1～12（預設 4），單獨即可成立篩選。
#[test]
fn screening_major_change_boundaries() {
    for (minimum, weeks, ok) in [
        (Some(-100.0), None, true),
        (Some(100.0), Some(12), true),
        (Some(0.5), Some(1), true),
        (Some(100.1), None, false),
        (Some(f64::NAN), None, false),
        (Some(0.0), Some(0), false),
        (Some(0.0), Some(13), false),
    ] {
        let mut params = screening_params();
        params.min_major_change = minimum;
        params.major_change_weeks = weeks;
        let validated = validate_screening_params(&params);
        assert_eq!(validated.is_ok(), ok, "{minimum:?} {weeks:?}");
        if let Ok(validated) = validated {
            assert!(validated.min_major_change.is_some());
            assert_eq!(validated.major_change_weeks, weeks.unwrap_or(4));
        }
    }
    // 只給回看週數不算篩選條件。
    let mut weeks_only = screening_params();
    weeks_only.major_change_weeks = Some(2);
    assert_eq!(
        validate_screening_params(&weeks_only).err(),
        Some("至少需要一個篩選條件")
    );
}
