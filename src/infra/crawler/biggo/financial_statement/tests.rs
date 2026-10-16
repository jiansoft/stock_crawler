//! BigGo 三大財報的解析與合併測試。

use rust_decimal_macros::dec;
use serde::de::DeserializeOwned;

use super::merge::{merge_balance, merge_income, parse_cash_flow, parse_period};
use super::rows::{
    AssetsRow, CashFlowRow, EpsRow, EquityRow, LiabilitiesRow, ProfitRow, Typed, typed_rows,
};
use super::*;
use crate::infra::crawler::biggo::Envelope;

/// 解析 fixture 的回應外層並取出 `data`。
fn data<T: DeserializeOwned>(json: &str) -> T {
    let envelope: Envelope<T> = serde_json::from_str(json).expect("fixture should parse");
    assert!(envelope.result);
    envelope.data.expect("fixture should have data")
}

fn period(year: i32, quarter: Option<u8>) -> FiscalPeriod {
    FiscalPeriod { year, quarter }
}

#[test]
fn parse_period_accepts_only_known_formats() {
    assert_eq!(
        parse_period("2026 Q2", ReportKind::Single).unwrap(),
        period(2026, Some(2))
    );
    assert_eq!(
        parse_period("2025", ReportKind::Annual).unwrap(),
        period(2025, None)
    );
    for (raw, kind) in [
        ("2026Q2", ReportKind::Single),
        ("2026 Q5", ReportKind::Single),
        ("26 Q1", ReportKind::Single),
        ("2026", ReportKind::Single),
        ("2026 Q2", ReportKind::Annual),
        ("abcd", ReportKind::Annual),
    ] {
        assert!(
            parse_period(raw, kind).is_err(),
            "{raw} ({kind:?}) 應被拒收"
        );
    }
}

/// 2330 2026Q2：損益與 EPS 依期別合併，數字與官方財報（及 Yahoo）一致。
#[test]
fn merge_income_joins_profit_and_eps_by_period() {
    let profit: Typed<ProfitRow> = data(include_str!("../testdata/profit_single_2330.json"));
    let eps: Typed<EpsRow> = data(include_str!("../testdata/eps_single_2330.json"));
    let statements =
        merge_income("2330", ReportKind::Single, profit.data, eps.data).expect("merge");

    assert_eq!(statements.len(), 3);
    let latest = statements.last().expect("latest");
    assert_eq!(latest.stock_symbol, "2330");
    assert_eq!(latest.period, period(2026, Some(2)));
    assert_eq!(latest.items.revenue, Some(1_270_380_250_000));
    assert_eq!(latest.items.gross_profit, Some(860_310_695_000));
    assert_eq!(latest.items.operating_expenses, Some(98_982_083_000));
    assert_eq!(latest.items.operating_income, Some(766_602_651_000));
    assert_eq!(latest.items.income_after_taxes, Some(706_780_923_000));
    assert_eq!(latest.items.eps, Some(dec!(27.25)));
}

/// 只有 EPS 的期別仍保留，缺的損益項目為 `None`。
#[test]
fn merge_income_keeps_periods_present_on_one_side() {
    let eps = vec![EpsRow {
        period: "2026 Q3".to_string(),
        eps: Some(1.5),
    }];
    let statements = merge_income("2330", ReportKind::Single, Vec::new(), eps).expect("merge");
    assert_eq!(statements.len(), 1);
    assert_eq!(statements[0].items.revenue, None);
    assert_eq!(statements[0].items.eps, Some(dec!(1.5)));
}

/// 年度資料的當年度是今年至今的累計，沒有第四季單季資料的年度要濾掉。
#[test]
fn retain_completed_years_drops_the_year_to_date_total() {
    let profit: Typed<ProfitRow> = data(include_str!("../testdata/profit_annual_2330.json"));
    let eps: Typed<EpsRow> = data(include_str!("../testdata/eps_annual_2330.json"));
    let annual = merge_income("2330", ReportKind::Annual, profit.data, eps.data).unwrap();
    assert_eq!(annual.len(), 2, "fixture 含 2026（累計）與 2025");

    let singles = [
        period(2026, Some(2)),
        period(2026, Some(1)),
        period(2025, Some(4)),
    ];
    let completed = retain_completed_years(annual, &singles);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].period, period(2025, None));
    assert_eq!(completed[0].items.eps, Some(dec!(66.26)));
    assert_eq!(completed[0].items.revenue, Some(3_809_054_272_000));
}

#[test]
fn parse_cash_flow_reads_all_items() {
    let rows: Typed<CashFlowRow> = data(include_str!("../testdata/cash_flow_single_2330.json"));
    let statements = parse_cash_flow("2330", ReportKind::Single, rows.data).unwrap();
    let latest = statements.last().expect("latest");
    assert_eq!(latest.period, period(2026, Some(2)));
    assert_eq!(latest.items.operating_cash_flow, Some(783_364_977_000));
    assert_eq!(latest.items.investing_cash_flow, Some(-492_810_418_000));
    assert_eq!(latest.items.financing_cash_flow, Some(-184_653_221_000));
    assert_eq!(latest.items.free_cash_flow, Some(290_554_559_000));
    assert_eq!(latest.items.net_cash_flow, Some(98_580_985_000));

    let annual: Typed<CashFlowRow> = data(include_str!("../testdata/cash_flow_annual_2330.json"));
    let annual = parse_cash_flow("2330", ReportKind::Annual, annual.data).unwrap();
    assert_eq!(annual.last().unwrap().period, period(2025, None));
    assert_eq!(
        annual.last().unwrap().items.operating_cash_flow,
        Some(2_274_975_625_000)
    );
}

/// 資產、負債、權益三份明細依期別合併成一張資產負債表。
#[test]
fn merge_balance_joins_three_breakdowns() {
    let assets: Vec<AssetsRow> = data(include_str!("../testdata/total_assets_2330.json"));
    let liabilities: Vec<LiabilitiesRow> = data(include_str!("../testdata/liabilities_2330.json"));
    let equity: Vec<EquityRow> = data(include_str!("../testdata/equity_2330.json"));
    let statements = merge_balance("2330", assets, liabilities, equity).unwrap();

    assert_eq!(statements.len(), 2);
    let latest = &statements.last().unwrap().items;
    assert_eq!(latest.cash_and_cash_equivalents, Some(3_134_218_213_000));
    assert_eq!(latest.current_financial_assets_fvtpl, Some(226_375_000));
    assert_eq!(latest.inventories, Some(385_524_542_000));
    assert_eq!(latest.current_assets, Some(4_565_700_742_000));
    assert_eq!(
        latest.investment_accounted_equity_method,
        Some(18_126_371_000)
    );
    assert_eq!(latest.property_plant_equipment, Some(4_302_880_478_000));
    assert_eq!(latest.total_assets, Some(9_375_654_727_000));
    assert_eq!(latest.current_liabilities, Some(1_857_761_825_000));
    assert_eq!(latest.bonds_payable, Some(815_036_716_000));
    assert_eq!(latest.total_liabilities, Some(2_901_183_746_000));
    assert_eq!(latest.capital_stock, Some(259_323_701_000));
    assert_eq!(latest.equity, Some(6_474_470_981_000));
}

/// 查無資料時 BigGo 回 `[]` 或 `null`，都視為空；非空陣列不是預期格式，要拒收。
#[test]
fn typed_rows_treats_empty_array_as_no_data() {
    let empty: Vec<EpsRow> = typed_rows(Some(serde_json::json!([]))).unwrap();
    assert!(empty.is_empty());
    let none: Vec<EpsRow> = typed_rows(None).unwrap();
    assert!(none.is_empty());

    let rows: Vec<EpsRow> = typed_rows(Some(serde_json::json!({
        "type": "single",
        "data": [{"period": "2026 Q2", "eps": 1.2}]
    })))
    .unwrap();
    assert_eq!(rows.len(), 1);

    let unexpected: Result<Vec<EpsRow>> =
        typed_rows(Some(serde_json::json!([{"period": "2026 Q2"}])));
    assert!(unexpected.is_err());
}

/// 金額接受整數與小數部分為 0 的數字，`null` 為 `None`，有小數一律拒收。
#[test]
fn amount_accepts_only_integral_numbers() {
    let parse = |json: &str| serde_json::from_str::<CashFlowRow>(json);
    let row =
        parse(r#"{"period":"2026 Q2","operating_cash_flow":1000.0,"net_cash_flow":null}"#).unwrap();
    assert_eq!(row.operating_cash_flow, Some(1000));
    assert_eq!(row.net_cash_flow, None);
    assert_eq!(row.free_cash_flow, None, "缺欄位為 None");
    assert!(parse(r#"{"period":"2026 Q2","operating_cash_flow":1000.5}"#).is_err());
    assert!(parse(r#"{"period":"2026 Q2","operating_cash_flow":"1000"}"#).is_err());
}
