//! 回測 SQL 的結構檢查，與相同 RS 分數時候選池排序和排行榜名次一致（B04）。

use super::*;

/// 兩種 SQL 都以 `WITH RECURSIVE` 開頭（交易日用 loose index scan）、含大盤濾網 `$11`、續抱門檻 `$12`、市場廣度門檻 `$13`、換股日偏移 `$14`
/// 、流動性門檻 `$15`、排序方式 `$16`、加權方式 `$17`、排除產業 `$18` 與基本面門檻 `$19`～`$23`，籌碼版另含
/// 連續買超的 CTE 與 `$24`／`$25`；沒有籌碼的版本排序用的連續買超欄位恆為 `NULL`。
#[test]
fn backtest_sql_variants_are_well_formed() {
    assert!(
        BACKTEST_SQL
            .trim_start()
            .starts_with("WITH RECURSIVE\nday_scan AS (")
    );
    assert!(BACKTEST_SQL.contains("FROM day_scan s\n    WHERE s.date <= $2::date + 400"));
    assert!(BACKTEST_SQL.contains("lr.liquidity_start,"));
    assert!(BACKTEST_SQL.contains("COALESCE($11::int, 1)"));
    assert!(BACKTEST_SQL.contains("rs_score >= $12"));
    assert!(BACKTEST_SQL.contains("$13::int IS NULL OR br.above"));
    assert!(BACKTEST_SQL.contains("WHERE i >= $14::int AND (i - $14::int) % $4::int = 0"));
    assert!(BACKTEST_SQL.contains("$15::int8 IS NULL OR"));
    assert!(BACKTEST_SQL.contains("BETWEEN f.liquidity_start AND f.date"));
    assert!(BACKTEST_SQL.contains("$15::int8 IS NOT NULL OR $16::text = 'trade_value'"));
    assert!(BACKTEST_SQL.contains("ORDER BY CASE $16::text"));
    assert!(BACKTEST_SQL.contains("NULL::float8 AS foreign_buy, NULL::float8 AS trust_buy"));
    assert!(BACKTEST_SQL.contains("OR $17::text <> 'equal' THEN ("));
    // 續抱名單不查成交金額，改在選定持股後補查（`HOLD_TRADE_VALUES_SQL`）。
    assert!(!BACKTEST_SQL.contains("CASE WHEN $17::text <> 'equal' THEN ("));
    assert!(
        BACKTEST_SQL.contains("u.stock_industry_id,\n               NULL::float8 AS trade_value")
    );
    assert!(BACKTEST_SQL.contains("AS candidate_trade_values"));
    assert!(BACKTEST_SQL.contains("AS hold_trade_values"));
    assert!(BACKTEST_SQL.contains("NOT si.name = ANY($18)"));
    assert!(BACKTEST_SQL.contains("$19::float8 IS NULL OR rv.revenue_yoy >= $19"));
    assert!(BACKTEST_SQL.contains("$20::smallint IS NULL OR rv.revenue_yoy_streak >= $20"));
    assert!(BACKTEST_SQL.contains("$21::float8 IS NULL OR rv.revenue_yoy_3m >= $21"));
    assert!(BACKTEST_SQL.contains("$22::float8 IS NULL OR fs.earnings_per_share >= $22"));
    assert!(BACKTEST_SQL.contains("$23::float8 IS NULL OR fs.return_on_equity >= $23"));
    assert!(BACKTEST_SQL.contains("HAVING MAX(ord) >= u.revenue_ord - 2"));
    assert!(BACKTEST_SQL.contains("ELSE make_date(fs.year::int + 1, 4, 1)"));
    assert!(BACKTEST_SQL.contains("$24::float8 IS NULL OR ym.gross_margin >= $24"));
    assert!(BACKTEST_SQL.contains("$25::float8 IS NULL OR ym.operating_margin >= $25"));
    assert!(BACKTEST_SQL.contains("NOT $26::bool OR ym.margins_up IS TRUE"));
    assert!(BACKTEST_SQL.contains("$27::float8 IS NULL OR bs.debt_ratio <= $27"));
    assert!(BACKTEST_SQL.contains("NOT $28::bool OR rr.is_record IS TRUE"));
    assert!(BACKTEST_SQL.contains("rr.month = rv.latest_month"));
    assert!(BACKTEST_SQL.contains("$29::float8 IS NULL OR rv.monthly_revenue >= $29"));
    assert!(BACKTEST_SQL.contains("MAX(monthly) FILTER (WHERE rn = 1) AS monthly_revenue"));
    assert!(BACKTEST_SQL.contains("NOT $30::bool OR cf.free_cash_flow_ttm > 0"));
    assert!(BACKTEST_SQL.contains("WHEN COUNT(c.free_cash_flow) = 4 THEN SUM(c.free_cash_flow)"));
    assert!(BACKTEST_SQL.contains("BETWEEN latest.ord - 3 AND latest.ord"));
    assert!(BACKTEST_SQL.contains("$31::float8 IS NULL OR qv.pe <= $31"));
    assert!(BACKTEST_SQL.contains("$32::float8 IS NULL OR qv.pb <= $32"));
    assert!(BACKTEST_SQL.contains("CASE WHEN q.\"PriceEarningRatio\" > 0 THEN"));
    assert!(
        BACKTEST_SQL.contains("OR $16::text IN ('pe_ratio', 'pb_ratio', 'magic_formula', 'peg')")
    );
    // PEG：8 季相連、前四季合計 > 0；條件包 COALESCE，與神奇公式對上同一個已公布的季。
    assert!(BACKTEST_SQL.contains("eps_growth AS MATERIALIZED ("));
    assert!(BACKTEST_SQL.contains("CASE WHEN quarters = 8 AND prev_ttm > 0"));
    assert!(
        BACKTEST_SQL.contains("previous AS (by_quarter RANGE BETWEEN 7 PRECEDING AND 4 PRECEDING)")
    );
    assert!(BACKTEST_SQL.contains("eg.ord = pq.ord"));
    assert!(BACKTEST_SQL.contains("mi.ord = pq.ord"));
    assert!(BACKTEST_SQL.contains(
        "qv.pe > 0 AND eg.growth > 0 AND eg.growth <= 100 AND qv.pe / eg.growth <= $50, FALSE)"
    ));
    assert!(BACKTEST_SQL.contains("WHEN 'peg' THEN -peg"));
    // 低波動：換股當天的風險指標以主鍵查，條件包 COALESCE。
    assert!(BACKTEST_SQL.contains("FROM daily_risk_metric r"));
    assert!(BACKTEST_SQL.contains("AND r.date = u.date"));
    assert!(BACKTEST_SQL.contains("COALESCE(rk.volatility <= $51, FALSE)"));
    assert!(BACKTEST_SQL.contains("COALESCE(rk.beta <= $52, FALSE)"));
    assert!(BACKTEST_SQL.contains("WHEN 'low_volatility' THEN -volatility"));
    // 神奇公式：只算起日前兩年以後、四季相連的營業利益；換股日對上已過法定期限的同一季。
    assert!(BACKTEST_SQL.contains("magic_inputs AS MATERIALIZED ("));
    assert!(BACKTEST_SQL.contains("RANGE BETWEEN 3 PRECEDING AND CURRENT ROW"));
    assert!(BACKTEST_SQL.contains("WHERE e.quarters = 4"));
    assert!(BACKTEST_SQL.contains("AND fiscal_year >= EXTRACT(YEAR FROM $1::date)::int - 2"));
    assert!(
        BACKTEST_SQL
            .contains("WHEN u.date >= make_date(EXTRACT(YEAR FROM u.date)::int, 4, 1) THEN 0")
    );
    assert!(BACKTEST_SQL.contains("WHEN 'magic_formula' THEN -magic_rank::float8"));
    assert!(BACKTEST_SQL.contains("WHEN 'pe_ratio' THEN -pe"));
    assert!(BACKTEST_SQL.contains("AND q.\"Date\" = u.date"));
    assert!(
        BACKTEST_SQL
            .contains("(($33::float8 IS NULL AND $34::int IS NULL) OR vl.cheap IS NOT FALSE)")
    );
    assert!(BACKTEST_SQL.contains("LEFT JOIN valuation_level vl ON vl.date = b.date"));
    assert!(BACKTEST_SQL.contains("h.below * 100 < $34::int * h.samples"));
    assert!(BACKTEST_SQL.contains("p.date < cur.date AND p.date >= cur.date - INTERVAL '5 years'"));
    assert!(!BACKTEST_SQL.contains("$53"));
    assert!(BACKTEST_SQL.contains("(hc.percent_400 - hb.percent_400)::float8 >= $48"));
    assert!(BACKTEST_SQL.contains("mg.margin_change <= $49"));
    assert!(BACKTEST_SQL.contains("OFFSET 19"));
    assert!(BACKTEST_SQL.contains("AND (NOT $47::bool OR tt.passed IS TRUE)"));
    assert!(BACKTEST_SQL.contains("q.\"ClosingPrice\" >= q.minimum_price_in_year * 1.3 AS passed"));
    assert!(BACKTEST_SQL.contains("rv.revenue_accel_months >= $45"));
    assert!(BACKTEST_SQL.contains("dg.growth_years >= $46"));
    assert!(BACKTEST_SQL.contains("),\ndividend_growth AS MATERIALIZED ("));
    assert!(BACKTEST_SQL.contains("dg.yr = EXTRACT(YEAR FROM u.date)::int - 1"));
    assert!(BACKTEST_SQL.contains("LEAD(yoy) OVER (ORDER BY ord DESC) AS prev_yoy"));
    assert!(BACKTEST_SQL.contains(") >= $40)"));
    assert!(BACKTEST_SQL.contains("dv.dividend_yield >= $41"));
    assert!(BACKTEST_SQL.contains("dv.fill_rate >= $42"));
    assert!(BACKTEST_SQL.contains("dv.avg_fill_days <= $43"));
    assert!(BACKTEST_SQL.contains("dv.dividend_years >= $44"));
    assert!(BACKTEST_SQL.contains("AND f.mature_date <= u.date"));
    assert!(BACKTEST_SQL.contains("l.ex_date <= u.date"));
    assert!(BACKTEST_SQL.contains("make_date(fh.fiscal_year + 1, 4, 1) <= u.date"));
    assert!(BACKTEST_SQL.contains("fh.fiscal_year >= EXTRACT(YEAR FROM u.date)::int - 2"));
    assert!(BACKTEST_SQL.contains("fy.eps_yoy >= $37"));
    assert!(BACKTEST_SQL.contains("ae.annual_growth >= $38"));
    assert!(BACKTEST_SQL.contains("n.date BETWEEN u.date - $39::int AND u.date"));
    assert!(BACKTEST_SQL.contains("make_date(a.year::int + 1, 4, 1) <= u.date"));
    assert!(
        BACKTEST_SQL
            .contains("(b.exit_date IS NULL OR b.exit_date > $2 OR b.samples = 0) AS pending")
    );
    assert!(BACKTEST_SQL.contains("LEAD(date, $4::int) OVER (ORDER BY date) AS exit_date"));
    assert!(BACKTEST_SQL.contains("WHEN rb.exit_date IS NOT NULL AND rb.exit_date <= $2 THEN"));
    assert!(BACKTEST_SQL.contains("OR b.date = (SELECT MAX(date) FROM rebal)"));
    assert!(
        BACKTEST_WITH_CHIP_SQL
            .trim_start()
            .starts_with("WITH RECURSIVE\nchip AS (")
    );
    assert!(BACKTEST_WITH_CHIP_SQL.contains("),\nday_scan AS ("));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("c.foreign_buy >= $53"));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("c.trust_buy >= $54"));
    assert!(!BACKTEST_WITH_CHIP_SQL.contains("$55"));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("LEFT JOIN chip c ON"));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("COALESCE(c.foreign_buy, 0)::float8 AS foreign_buy"));
    assert!(BACKTEST_SQL.contains("rs_score DESC, rs_rank ASC, stock_symbol"));
    assert!(BACKTEST_SQL.contains(
        "array_agg(stock_symbol::text ORDER BY rs_score DESC, rs_rank ASC, stock_symbol)"
    ));
    for rank_by in RANK_BY {
        assert!(rank_by == "rs_score" || BACKTEST_SQL.contains(&format!("WHEN '{rank_by}' THEN")));
    }
}

/// 驗證 B04：當多檔股票 rs_score 相同（例如全體前幾檔 rs_score 均為 99）時，
/// 回測候選池排序（rs_score DESC, rs_rank ASC, stock_symbol）與 score_universe 排行榜名次順序一致，
/// 不會因為股票代號字母順序把後段排到前面。
#[test]
fn rs_rank_tie_break_aligns_with_rankings() {
    use crate::domain::quote::relative_strength::{UniverseRow, score_universe};

    let rows = vec![
        UniverseRow {
            stock_symbol: "3441".to_string(),
            name: "聯德".to_string(),
            market_id: 2,
            industry: "電子零組件業".to_string(),
            return_20d: Some(15.0),
            return_60d: 50.0,
            return_120d: 40.0,
            return_240d: 30.0,
            closing_price: Some(100.0),
        },
        UniverseRow {
            stock_symbol: "6225".to_string(),
            name: "天瀚".to_string(),
            market_id: 2,
            industry: "光電業".to_string(),
            return_20d: Some(12.0),
            return_60d: 48.0,
            return_120d: 38.0,
            return_240d: 28.0,
            closing_price: Some(50.0),
        },
        UniverseRow {
            stock_symbol: "6213".to_string(),
            name: "聯茂".to_string(),
            market_id: 2,
            industry: "電子零組件業".to_string(),
            return_20d: Some(10.0),
            return_60d: 45.0,
            return_120d: 35.0,
            return_240d: 25.0,
            closing_price: Some(80.0),
        },
        UniverseRow {
            stock_symbol: "6672".to_string(),
            name: "騰雲".to_string(),
            market_id: 2,
            industry: "資訊服務業".to_string(),
            return_20d: Some(8.0),
            return_60d: 40.0,
            return_120d: 30.0,
            return_240d: 20.0,
            closing_price: Some(120.0),
        },
        UniverseRow {
            stock_symbol: "1303".to_string(),
            name: "南亞".to_string(),
            market_id: 2,
            industry: "塑膠工業".to_string(),
            return_20d: Some(5.0),
            return_60d: 35.0,
            return_120d: 25.0,
            return_240d: 15.0,
            closing_price: Some(60.0),
        },
    ];

    let scored = score_universe(rows);
    let ranked_symbols: Vec<&str> = scored.iter().map(|s| s.row.stock_symbol.as_str()).collect();
    assert_eq!(ranked_symbols, ["3441", "6225", "6213", "6672", "1303"]);

    // 模擬 backtest picks 的排序：(rs_score DESC, rs_rank ASC, stock_symbol)
    let mut picks: Vec<_> = scored
        .iter()
        .map(|s| (s.rs_score, s.rank, s.row.stock_symbol.clone()))
        .collect();
    picks.reverse();
    picks.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    let pick_symbols: Vec<&str> = picks.iter().map(|p| p.2.as_str()).collect();
    // 必須與 score_universe 排行榜完全一致，而非依 stock_symbol ASC 排成 ["1303", "3441", "6213", "6225", "6672"]
    assert_eq!(pick_symbols, ranked_symbols);
}
