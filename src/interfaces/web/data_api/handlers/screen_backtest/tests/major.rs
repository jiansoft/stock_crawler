//! 千張大戶持股比例變化條件：參數範圍、起日檢查與 SQL 片段。

use super::super::validate::validate_major_change_period;
use super::*;

fn params(query: &str) -> ScreenBacktestParams {
    let uri: axum::http::Uri = format!("/?{query}").parse().expect("有效網址");
    axum::extract::Query::<ScreenBacktestParams>::try_from_uri(&uri)
        .expect("可解析的參數")
        .0
}

/// 變化下限 -100～100、回看週數 1～12；邊界值通過，超出範圍回錯誤訊息。
#[test]
fn validate_major_change_ranges() {
    assert_eq!(
        validate_fundamentals(&params("min_major_change=-100&major_change_weeks=12")),
        Ok(())
    );
    assert_eq!(
        validate_fundamentals(&params("min_major_change=100&major_change_weeks=1")),
        Ok(())
    );
    for (query, message) in [
        (
            "min_major_change=100.5",
            "min_major_change 必須介於 -100 至 100",
        ),
        (
            "min_major_change=-101",
            "min_major_change 必須介於 -100 至 100",
        ),
        (
            "major_change_weeks=0",
            "major_change_weeks 必須介於 1 至 12",
        ),
        (
            "major_change_weeks=13",
            "major_change_weeks 必須介於 1 至 12",
        ),
    ] {
        assert_eq!(
            validate_fundamentals(&params(query)),
            Err(message),
            "{query}"
        );
    }
}

/// 起日要晚於第一週加回看週數（預設 4 週）；沒有設條件時不檢查，沒有任何大戶資料時回錯誤。
#[test]
fn major_change_period_needs_enough_history() {
    let first = Some(day(2023, 7, 14));
    assert_eq!(
        validate_major_change_period(&params(""), day(2019, 1, 2), None),
        Ok(())
    );
    let default_weeks = params("min_major_change=0");
    assert_eq!(
        validate_major_change_period(&default_weeks, day(2023, 8, 12), first),
        Ok(())
    );
    let early = validate_major_change_period(&default_weeks, day(2023, 8, 11), first)
        .expect_err("起日太早");
    assert!(
        early.contains("from 必須晚於 2023-08-11") && early.contains("2023-07-14"),
        "{early}"
    );
    let twelve = params("min_major_change=0&major_change_weeks=12");
    assert!(validate_major_change_period(&twelve, day(2023, 10, 6), first).is_err());
    assert_eq!(
        validate_major_change_period(&twelve, day(2023, 10, 7), first),
        Ok(())
    );
    assert!(
        validate_major_change_period(&default_weeks, day(2024, 1, 2), None)
            .expect_err("沒有大戶資料")
            .contains("沒有任何一週")
    );
}

/// 四種回測 SQL 都只看日期早於換股日的資料週，並以往前第 `$36` 週相減；籌碼條件改用 `$37`／`$38`。
#[test]
fn backtest_sql_uses_only_published_holder_weeks() {
    for sql in [
        BACKTEST_SQL,
        BACKTEST_WITH_CHIP_SQL,
        super::super::sql::BACKTEST_NEXT_OPEN_SQL,
        super::super::sql::BACKTEST_WITH_CHIP_NEXT_OPEN_SQL,
    ] {
        assert!(sql.contains(r#"WHERE "date" < rb.date"#));
        assert!(sql.contains("bw.wk = cw.wk - $36::int"));
        assert!(sql.contains(r#"cw."date" >= rb.date - 14"#));
        assert!(sql.contains("(hc.major_percent - hb.major_percent)::float8 >= $35"));
    }
    assert!(!BACKTEST_SQL.contains("$38"));
}
