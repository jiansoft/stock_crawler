//! 持股 Data API 端點的單元測試。

use rust_decimal_macros::dec;

use super::*;
use crate::domain::portfolio::health::FundamentalSignals;

/// 報價日取最新、各欄位原樣轉入領域彙總。
#[test]
fn build_response_uses_latest_quote_date() {
    let row = |symbol: &str, day: u32, price: Option<Decimal>| PositionRow {
        stock_symbol: symbol.to_owned(),
        shares: 1_000,
        cost: dec!(10_000),
        dividends_received: dec!(120),
        name: "測試".to_owned(),
        industry_id: Some(1),
        industry_name: "水泥工業".to_owned(),
        quote_date: NaiveDate::from_ymd_opt(2026, 4, day),
        closing_price: price,
        dividend_yield_percent: Some(dec!(3.5)),
        f_score: Some(7),
        pe_ratio: Some(dec!(15)),
        pb_ratio: None,
    };
    let response = build_response(
        2,
        vec![row("1101", 29, Some(dec!(12))), row("1102", 30, None)],
        Vec::new(),
    );
    assert_eq!(response.member_id, 2);
    assert_eq!(response.data_as_of.as_deref(), Some("2026-04-30"));
    assert_eq!((response.symbols, response.market_value), (2, 12_000.0));
    assert_eq!(response.dividends_received, 240.0);
    assert_eq!(response.holdings[0].stock_symbol, "1101");
    assert_eq!(response.holdings[0].f_score, Some(7));
    assert_eq!(
        (response.holdings[0].pe_ratio, response.holdings[0].pb_ratio),
        (Some(15.0), None)
    );
    // 只有 1101 有市值：組合本益比就是它的 15 倍。
    assert_eq!(response.weighted_pe.map(|metric| metric.value), Some(15.0));
    assert_eq!(response.holdings[1].closing_price, None);
    assert_eq!(response.industries[0].symbols, 2);

    let empty = build_response(0, Vec::new(), Vec::new());
    assert!(empty.data_as_of.is_none() && empty.holdings.is_empty());
}

/// 依代號附上分數與連續天數；沒有分數或籌碼的持股維持 null；資料日只在有資料時填入。
#[test]
fn attach_indicators_matches_by_symbol() {
    use super::super::relative_strength::UniverseRow;

    let position = |symbol: &str| PositionRow {
        stock_symbol: symbol.to_owned(),
        shares: 1_000,
        cost: dec!(10_000),
        dividends_received: dec!(0),
        name: symbol.to_owned(),
        industry_id: None,
        industry_name: String::new(),
        quote_date: NaiveDate::from_ymd_opt(2026, 10, 14),
        closing_price: Some(dec!(10)),
        dividend_yield_percent: None,
        f_score: None,
        pe_ratio: None,
        pb_ratio: None,
    };
    let scored = |symbol: &str, rs_score: u8| ScoredStock {
        rank: 1,
        rs_score,
        weighted: 1.0,
        row: UniverseRow {
            stock_symbol: symbol.to_owned(),
            name: String::new(),
            market_id: 2,
            industry: String::new(),
            return_20d: None,
            return_60d: 0.0,
            return_120d: 0.0,
            return_240d: 0.0,
            closing_price: None,
        },
    };
    let day = |d: u32| NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期");
    let mut response = build_response(1, vec![position("0050"), position("2330")], Vec::new());
    attach_indicators(
        &mut response,
        Some((day(14), vec![scored("2330", 79), scored("2317", 50)])),
        Some((
            day(13),
            vec![StreakRow {
                stock_symbol: "2330".to_owned(),
                foreign_streak: -6,
                trust_streak: 11,
            }],
        )),
    );
    assert_eq!(response.rs_as_of.as_deref(), Some("2026-10-14"));
    assert_eq!(response.chip_as_of.as_deref(), Some("2026-10-13"));
    let by_symbol = |symbol: &str| {
        response
            .holdings
            .iter()
            .find(|holding| holding.stock_symbol == symbol)
            .map(|holding| {
                (
                    holding.rs_score,
                    holding.foreign_streak,
                    holding.trust_streak,
                )
            })
    };
    assert_eq!(by_symbol("2330"), Some((Some(79), Some(-6), Some(11))));
    assert_eq!(by_symbol("0050"), Some((None, None, None)));

    let mut bare = build_response(1, vec![position("2330")], Vec::new());
    attach_indicators(&mut bare, None, None);
    assert!(bare.rs_as_of.is_none() && bare.chip_as_of.is_none());
    assert!(bare.holdings[0].rs_score.is_none());
}

/// 營收、三率與警示依代號填入持股：三率四捨五入到小數 2 位、三率三降與營收連 3 月年減發警示，
/// 沒有營收與財報的持股維持 `null` 與空警示；`warning_symbols` 只算有警示的檔數。
#[test]
fn build_response_attaches_fundamentals_and_warnings() {
    let position = |symbol: &str, price| PositionRow {
        stock_symbol: symbol.to_owned(),
        shares: 1_000,
        closing_price: Some(price),
        ..Default::default()
    };
    let warned = HoldingFundamentals {
        stock_symbol: "2330".to_owned(),
        revenue_month: Some(202609),
        revenue_yoy: Some(dec!(-12.5)),
        margin_period: Some((2026, 2)),
        margins: [Some(40.126), Some(-1.5), Some(2.0)],
        signals: FundamentalSignals {
            revenue_decline_months: Some(3),
            operating_margin: Some(-1.5),
            margin_changes: [Some(-4.874), Some(-4.5), Some(-2.0)],
            free_cash_flow_ttm: Some(-500),
        },
    };
    let response = build_response(
        0,
        vec![position("2330", dec!(100)), position("0050", dec!(50))],
        vec![warned],
    );
    assert_eq!(response.warning_symbols, 1);
    let holding = |symbol: &str| {
        response
            .holdings
            .iter()
            .find(|holding| holding.stock_symbol == symbol)
            .expect("holding")
    };
    let tsmc = holding("2330");
    assert_eq!(
        (
            tsmc.revenue_month.as_deref(),
            tsmc.revenue_yoy_percent,
            tsmc.revenue_decline_months
        ),
        (Some("2026-09"), Some(-12.5), Some(3))
    );
    assert_eq!(tsmc.margin_period.as_deref(), Some("2026-Q2"));
    assert_eq!(
        (
            tsmc.gross_margin_percent,
            tsmc.operating_margin_percent,
            tsmc.net_margin_percent
        ),
        (Some(40.13), Some(-1.5), Some(2.0))
    );
    assert_eq!(tsmc.margins_trend.as_deref(), Some("all_down"));
    assert_eq!(tsmc.free_cash_flow_ttm, Some(-500));
    assert_eq!(
        tsmc.warnings,
        [
            "revenue_decline",
            "margins_all_down",
            "operating_loss",
            "negative_free_cash_flow"
        ]
    );
    let etf = holding("0050");
    assert!(etf.revenue_month.is_none() && etf.margin_period.is_none());
    assert!(etf.margins_trend.is_none() && etf.warnings.is_empty());
}
