//! 基本面條件與排除產業的參數驗證。

use super::*;

/// 基本面門檻：未設定與邊界值通過，超出範圍、非數值或連續月數為 0 回錯誤訊息。
#[test]
fn validate_fundamentals_checks_ranges() {
    let params = |query: &str| {
        let uri: axum::http::Uri = format!("/?{query}").parse().expect("有效網址");
        axum::extract::Query::<ScreenBacktestParams>::try_from_uri(&uri)
            .expect("可解析的參數")
            .0
    };
    assert_eq!(validate_fundamentals(&params("")), Ok(()));
    assert_eq!(
        validate_fundamentals(&params(
            "min_revenue_yoy_percent=-100&min_revenue_yoy_streak=12&min_revenue_yoy_3m_percent=10000\
             &min_eps=-10000&min_roe_percent=10000&min_gross_margin_percent=100\
             &min_operating_margin_percent=-10000&max_debt_ratio_percent=0&margins_yoy_up=true\
             &min_monthly_revenue=0&max_pe=10000&max_pb=0"
        )),
        Ok(())
    );
    for (query, message) in [
        (
            "min_revenue_yoy_percent=-100.5",
            "min_revenue_yoy_percent 必須介於 -100 至 10000",
        ),
        (
            "min_revenue_yoy_percent=NaN",
            "min_revenue_yoy_percent 必須介於 -100 至 10000",
        ),
        (
            "min_revenue_yoy_streak=0",
            "min_revenue_yoy_streak 必須介於 1 至 12",
        ),
        (
            "min_revenue_yoy_streak=13",
            "min_revenue_yoy_streak 必須介於 1 至 12",
        ),
        (
            "min_revenue_yoy_3m_percent=10001",
            "min_revenue_yoy_3m_percent 必須介於 -100 至 10000",
        ),
        ("max_pe=-1", "max_pe 必須介於 0 至 10000"),
        ("max_pe=10001", "max_pe 必須介於 0 至 10000"),
        ("max_pb=1001", "max_pb 必須介於 0 至 1000"),
        ("max_pb=NaN", "max_pb 必須介於 0 至 1000"),
        (
            "min_monthly_revenue=-1",
            "min_monthly_revenue 必須介於 0 至 1000000000000",
        ),
        (
            "min_monthly_revenue=1000000000001",
            "min_monthly_revenue 必須介於 0 至 1000000000000",
        ),
        ("min_eps=-10001", "min_eps 必須介於 -10000 至 10000"),
        (
            "min_roe_percent=inf",
            "min_roe_percent 必須介於 -10000 至 10000",
        ),
        (
            "min_gross_margin_percent=100.1",
            "min_gross_margin_percent 必須介於 -10000 至 100",
        ),
        (
            "min_operating_margin_percent=-10000.5",
            "min_operating_margin_percent 必須介於 -10000 至 100",
        ),
        (
            "max_debt_ratio_percent=1000.1",
            "max_debt_ratio_percent 必須介於 0 至 1000",
        ),
    ] {
        assert_eq!(
            validate_fundamentals(&params(query)),
            Err(message),
            "{query}"
        );
    }
}

/// 排除產業：逗號分隔、去掉空白與空項目、去除重複；超過 10 個回錯誤。
#[test]
fn parse_excluded_industries_splits_and_limits() {
    assert_eq!(parse_excluded_industries(None), Ok(Vec::new()));
    assert_eq!(parse_excluded_industries(Some(" , ")), Ok(Vec::new()));
    assert_eq!(
        parse_excluded_industries(Some(" 金融保險業,航運業, 金融保險業,")),
        Ok(vec!["金融保險業".to_owned(), "航運業".to_owned()])
    );
    let many: Vec<String> = (0..11).map(|i| format!("產業{i}")).collect();
    assert!(parse_excluded_industries(Some(&many.join(","))).is_err());
    assert_eq!(
        parse_excluded_industries(Some(&many[..10].join(","))).map(|names| names.len()),
        Ok(10)
    );
}
