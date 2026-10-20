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

/// 十四個排序欄位乘上兩種方向必須完整映射，指標降冪也要明確
/// `NULLS LAST`；不在白名單的文字必須在接觸 SQL 前遭拒。
#[test]
fn screening_sort_has_twenty_eight_static_branches() {
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
