//! 條件選股參數驗證、排序白名單與新鮮度公式的 deterministic tests。

use super::*;

/// `all` 不縮小市場集合，不能單獨通過；上市／上櫃或任何一個實質條件
/// 都可成立，固定預設排序與 limit 也應在驗證後補齊。
#[test]
fn screening_requires_a_meaningful_filter() {
    assert_eq!(
        validate_screening_params(&screening_params()).err(),
        Some("至少需要一個篩選條件")
    );
    let mut explicit_all = screening_params();
    explicit_all.market = Some("all".to_owned());
    assert!(validate_screening_params(&explicit_all).is_err());

    let mut twse = screening_params();
    twse.market = Some("twse".to_owned());
    let validated = validate_screening_params(&twse).expect("twse 本身是有效條件");
    assert_eq!(validated.market_id, 2);
    assert_eq!(validated.limit, 20);
    assert_eq!(validated.order_by, "ORDER BY stock_symbol ASC");

    let mut industry = screening_params();
    industry.industry_id = Some(24);
    assert!(validate_screening_params(&industry).is_ok());
    let mut minimum = screening_params();
    minimum.min_eps = Some(0.0);
    assert!(validate_screening_params(&minimum).is_ok());

    // 財務體質條件各自都算有效篩選；positive_free_cash_flow=false 等於沒有條件。
    let mut f_score = screening_params();
    f_score.min_f_score = Some(0);
    assert!(validate_screening_params(&f_score).is_ok());
    let mut debt = screening_params();
    debt.max_debt_ratio_percent = Some(50.0);
    assert!(validate_screening_params(&debt).is_ok());
    let mut cash_flow = screening_params();
    cash_flow.positive_free_cash_flow = Some(true);
    assert!(validate_screening_params(&cash_flow).is_ok());
    cash_flow.positive_free_cash_flow = Some(false);
    assert_eq!(
        validate_screening_params(&cash_flow).err(),
        Some("至少需要一個篩選條件")
    );
}

/// 產業代碼、估值區間、EPS／ROE 範圍與 limit 的錯誤訊息；合法的估值區間可單獨成立。
#[test]
fn screening_rejects_invalid_industry_band_and_limit() {
    let mut industry = screening_params();
    industry.industry_id = Some(0);
    assert_eq!(
        validate_screening_params(&industry).err(),
        Some("industry_id 必須為正整數")
    );

    let mut band = screening_params();
    band.valuation_band = Some("cheap".to_owned());
    assert_eq!(
        validate_screening_params(&band).err(),
        Some("valuation_band 不在允許範圍")
    );
    for value in [
        "undervalued",
        "fair_valued",
        "overvalued",
        "highly_overvalued",
    ] {
        band.valuation_band = Some(value.to_owned());
        assert!(validate_screening_params(&band).is_ok(), "{value}");
    }

    let mut eps = screening_params();
    eps.min_eps = Some(10_000.5);
    assert_eq!(
        validate_screening_params(&eps).err(),
        Some("min_eps 必須介於 -10000 至 10000")
    );
    let mut roe = screening_params();
    roe.min_roe_percent = Some(-10_000.5);
    assert_eq!(
        validate_screening_params(&roe).err(),
        Some("min_roe_percent 必須介於 -10000 至 10000")
    );

    for limit in [0, 51] {
        let mut params = screening_params();
        params.market = Some("twse".to_owned());
        params.limit = Some(limit);
        assert_eq!(
            validate_screening_params(&params).err(),
            Some("limit 必須介於 1 至 50"),
            "{limit}"
        );
    }
}

/// 營收動能條件：連續年增月數 1–12、近 3 月累計年增率 -100–10000，各自都算有效篩選。
#[test]
fn screening_revenue_momentum_boundaries() {
    for (months, ok) in [(0, false), (1, true), (12, true), (13, false)] {
        let mut params = screening_params();
        params.min_revenue_yoy_streak = Some(months);
        assert_eq!(validate_screening_params(&params).is_ok(), ok, "{months}");
    }
    for (value, ok) in [
        (-100.0, true),
        (10_000.0, true),
        (-100.5, false),
        (f64::NAN, false),
    ] {
        let mut params = screening_params();
        params.min_revenue_yoy_3m_percent = Some(value);
        let validated = validate_screening_params(&params);
        assert_eq!(validated.is_ok(), ok, "{value}");
        if let Ok(validated) = validated {
            assert!(validated.min_revenue_yoy_3m_percent.is_some());
        }
    }
}

/// 最低月營收：0 到一兆千元，單獨即可成立篩選；負數與非有限值回 422。
#[test]
fn screening_min_monthly_revenue_boundaries() {
    for (value, ok) in [
        (0.0, true),
        (1_000_000_000_000.0, true),
        (-1.0, false),
        (f64::INFINITY, false),
    ] {
        let mut params = screening_params();
        params.min_monthly_revenue = Some(value);
        let validated = validate_screening_params(&params);
        assert_eq!(validated.is_ok(), ok, "{value}");
        if let Ok(validated) = validated {
            assert!(validated.min_monthly_revenue.is_some());
        }
    }
}

/// 本益比上限 0–10000、股價淨值比上限 0–1000，單獨即可成立篩選；負數與非有限值回 422。
#[test]
fn screening_valuation_cap_boundaries() {
    for (pe, pb, ok) in [
        (Some(0.0), None, true),
        (Some(10_000.0), Some(1_000.0), true),
        (None, Some(0.5), true),
        (Some(-1.0), None, false),
        (Some(10_001.0), None, false),
        (None, Some(1_001.0), false),
        (None, Some(f64::NAN), false),
    ] {
        let mut params = screening_params();
        params.max_pe = pe;
        params.max_pb = pb;
        let validated = validate_screening_params(&params);
        assert_eq!(validated.is_ok(), ok, "{pe:?} {pb:?}");
        if let Ok(validated) = validated {
            assert_eq!(validated.max_pe.is_some(), pe.is_some());
            assert_eq!(validated.max_pb.is_some(), pb.is_some());
        }
    }
}

/// 財務體質條件的範圍：F 分數 0–9、毛利率上限 100%、負債比 0–1000%。
#[test]
fn screening_health_boundaries_are_deterministic() {
    let mut f_score = screening_params();
    f_score.min_f_score = Some(9);
    assert!(validate_screening_params(&f_score).is_ok());
    f_score.min_f_score = Some(10);
    assert_eq!(
        validate_screening_params(&f_score).err(),
        Some("min_f_score 必須介於 0 至 9")
    );
    for (value, ok) in [
        (100.0, true),
        (-10_000.0, true),
        (100.01, false),
        (f64::NAN, false),
    ] {
        let mut params = screening_params();
        params.min_gross_margin_percent = Some(value);
        assert_eq!(validate_screening_params(&params).is_ok(), ok, "{value}");
    }
    for (value, ok) in [
        (0.0, true),
        (1_000.0, true),
        (-0.01, false),
        (1_000.01, false),
    ] {
        let mut params = screening_params();
        params.max_debt_ratio_percent = Some(value);
        assert_eq!(validate_screening_params(&params).is_ok(), ok, "{value}");
    }
}

/// 十六個排序欄位乘上兩種方向必須完整映射，指標降冪也要明確
/// `NULLS LAST`；不在白名單的文字必須在接觸 SQL 前遭拒。
#[test]
fn screening_sort_has_thirty_two_static_branches() {
    let fields = [
        "stock_symbol",
        "revenue_yoy",
        "revenue_yoy_streak",
        "revenue_yoy_3m",
        "monthly_revenue",
        "eps",
        "roe",
        "dividend_yield",
        "valuation_percentage",
        "f_score",
        "gross_margin",
        "debt_ratio",
        "operating_margin",
        "rs_score",
        "pe_ratio",
        "pb_ratio",
    ];
    for field in fields {
        for order in ["asc", "desc"] {
            let sql = screen_order_by(field, order).expect("白名單組合應存在");
            assert!(sql.starts_with("ORDER BY "));
            if field != "stock_symbol" {
                assert!(sql.contains("NULLS LAST"));
                assert!(sql.ends_with("stock_symbol ASC"));
            }
        }
    }
    assert!(screen_order_by("closing_price; DROP TABLE stocks", "asc").is_err());
    assert!(screen_order_by("eps", "sideways").is_err());
}

/// 數值範圍包含契約兩端，超界、NaN 與無限大都必須回 422；這可避免
/// 非有限浮點數進入 Decimal 轉換或 PostgreSQL 比較。
#[test]
fn screening_numeric_boundaries_are_deterministic() {
    for value in [-100.0, 10_000.0] {
        let mut params = screening_params();
        params.min_revenue_yoy_percent = Some(value);
        assert!(validate_screening_params(&params).is_ok());
    }
    for value in [-100.01, 10_000.01, f64::NAN, f64::INFINITY] {
        let mut params = screening_params();
        params.min_revenue_yoy_percent = Some(value);
        assert!(validate_screening_params(&params).is_err());
    }
    let mut yield_at_zero = screening_params();
    yield_at_zero.min_dividend_yield_percent = Some(0.0);
    assert!(validate_screening_params(&yield_at_zero).is_ok());
    let mut negative_yield = screening_params();
    negative_yield.min_dividend_yield_percent = Some(-0.01);
    assert!(validate_screening_params(&negative_yield).is_err());
}

/// 新鮮度邊界使用月份／季度序號處理跨年，並以「差三月／兩季／三十日」
/// 為仍有效的最後一天；未來日期一律不視為新鮮。
#[test]
fn screening_freshness_boundaries_cross_years() {
    let query_date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    assert!(revenue_month_is_fresh(query_date, 202510));
    assert!(!revenue_month_is_fresh(query_date, 202509));
    assert!(!revenue_month_is_fresh(query_date, 202602));

    assert!(financial_period_is_fresh(query_date, 2025, "Q3"));
    assert!(!financial_period_is_fresh(query_date, 2025, "Q2"));
    assert!(!financial_period_is_fresh(query_date, 2026, "Q2"));
    assert!(!financial_period_is_fresh(query_date, 2025, ""));

    // F 分數：2026 年查詢時 2024、2025 年報仍有效（年報 3 月底前才公布）。
    assert!(f_score_year_is_fresh(query_date, 2025));
    assert!(f_score_year_is_fresh(query_date, 2024));
    assert!(!f_score_year_is_fresh(query_date, 2023));
    assert!(!f_score_year_is_fresh(query_date, 2026));

    let thirty_days_ago = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let thirty_one_days_ago = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
    assert!(analytical_date_is_fresh(query_date, thirty_days_ago));
    assert!(!analytical_date_is_fresh(query_date, thirty_one_days_ago));
    assert!(!analytical_date_is_fresh(
        query_date,
        NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
    ));
}

/// 平均成交金額門檻的範圍：1–100000 百萬元；單獨設定也算有篩選條件。
#[test]
fn validate_avg_trade_value_range() {
    for (value, ok) in [(0, false), (1, true), (100_000, true), (100_001, false)] {
        let mut params = screening_params();
        params.min_avg_trade_value = Some(value);
        match validate_screening_params(&params) {
            Ok(validated) => {
                assert!(ok, "{value} 應被拒絕");
                assert_eq!(
                    validated.min_avg_trade_value,
                    Some(Decimal::from(value) * Decimal::from(1_000_000))
                );
            }
            Err(message) => {
                assert!(!ok, "{value} 應通過");
                assert_eq!(
                    message,
                    "min_avg_trade_value 必須介於 1 至 100000（百萬元）"
                );
            }
        }
    }
}

/// 營業利益率門檻範圍 -10000–100；營業利益率與三率三升單獨設定都算有篩選條件，三率三升 `false` 不算。
#[test]
fn validate_margin_filters() {
    for (value, ok) in [
        (-10_000.5, false),
        (-10_000.0, true),
        (100.0, true),
        (100.5, false),
    ] {
        let mut params = screening_params();
        params.min_operating_margin_percent = Some(value);
        assert_eq!(validate_screening_params(&params).is_ok(), ok, "{value}");
    }
    let mut params = screening_params();
    params.margins_yoy_up = Some(true);
    assert!(validate_screening_params(&params).is_ok());
    params.margins_yoy_up = Some(false);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("至少需要一個篩選條件")
    );
    params.revenue_record = Some(true);
    assert!(validate_screening_params(&params).is_ok());
    params.revenue_record = Some(false);
    assert!(validate_screening_params(&params).is_err());
}

/// 排除產業 id：逗號分隔、去空白與重複，最多 10 個；非正整數、超過上限或包含 `industry_id` 回 422；
/// 單獨設定也算篩選條件。
#[test]
fn validate_exclude_industry_ids() {
    assert_eq!(parse_exclude_industry_ids(None), Ok(None));
    assert_eq!(parse_exclude_industry_ids(Some(" , ")), Ok(None));
    assert_eq!(
        parse_exclude_industry_ids(Some("17, 15,17")),
        Ok(Some(vec![17, 15]))
    );
    for bad in ["0", "-3", "金融", "1,2,3,4,5,6,7,8,9,10,11"] {
        assert!(parse_exclude_industry_ids(Some(bad)).is_err(), "{bad}");
    }
    let mut params = screening_params();
    params.exclude_industry_ids = Some("17".to_owned());
    let validated = validate_screening_params(&params).expect("單獨設定即可篩選");
    assert_eq!(validated.exclude_industry_ids, Some(vec![17]));
    params.industry_id = Some(17);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("exclude_industry_ids 不可包含 industry_id 指定的產業")
    );
}
