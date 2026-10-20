//! 財務體質快照（評分年度、最新一季指標與近四季自由現金流）的測試。

use super::*;

fn quarter(fiscal_year: i32, quarter: u8, free_cash_flow: Option<i64>) -> QuarterFigures {
    QuarterFigures {
        fiscal_year,
        quarter,
        has_income: true,
        revenue: Some(1_000),
        gross_profit: Some(400),
        total_assets: Some(8_000),
        total_liabilities: Some(2_000),
        free_cash_flow,
    }
}

/// 評分年度取最新一個有年度損益表的年度；季報指標取最新一個有損益表的季度，
/// 近四季自由現金流跨年相加。
#[test]
fn snapshot_scores_latest_year_and_quarter() {
    let (current, previous) = healthy_pair();
    let annual = BTreeMap::from([
        (
            2019,
            AnnualFigures {
                has_income: true,
                figures: previous,
            },
        ),
        (
            2020,
            AnnualFigures {
                has_income: true,
                figures: current,
            },
        ),
        // 只有資產負債表、還沒有年度損益表的年度不當評分年度。
        (
            2021,
            AnnualFigures {
                has_income: false,
                figures: YearFigures::default(),
            },
        ),
    ]);
    let mut balance_only = quarter(2021, 2, None);
    balance_only.has_income = false;
    let quarters = [
        quarter(2020, 3, Some(100)),
        quarter(2020, 4, Some(-50)),
        quarter(2021, 1, Some(200)),
        quarter(2020, 2, Some(10)),
        balance_only,
    ];

    let snapshot = snapshot("2330".to_owned(), &annual, &quarters);
    assert_eq!(snapshot.f_score_year, Some(2020));
    assert_eq!((snapshot.f_score, snapshot.f_score_evaluated), (Some(8), 9));
    assert_eq!(
        (snapshot.quarter_year, snapshot.quarter),
        (Some(2021), Some(1))
    );
    assert_eq!(snapshot.gross_margin_percent, Some(dec!(40)));
    assert_eq!(snapshot.debt_ratio_percent, Some(dec!(25)));
    assert_eq!(snapshot.free_cash_flow_ttm, Some(260));
}

/// 近四季缺任一季（含中間斷一季）就不算近四季自由現金流；沒有任何資料時全為 `None`。
#[test]
fn snapshot_requires_four_consecutive_quarters_for_free_cash_flow() {
    let gap = [
        quarter(2020, 1, Some(1)),
        quarter(2020, 2, Some(1)),
        quarter(2020, 4, Some(1)),
        quarter(2021, 1, Some(1)),
    ];
    assert_eq!(
        snapshot("2330".to_owned(), &BTreeMap::new(), &gap).free_cash_flow_ttm,
        None
    );
    let mut missing = quarter(2020, 4, None);
    missing.revenue = Some(0);
    let quarters = [
        quarter(2020, 1, Some(1)),
        quarter(2020, 2, Some(1)),
        quarter(2020, 3, Some(1)),
        missing,
    ];
    let partial = snapshot("2330".to_owned(), &BTreeMap::new(), &quarters);
    assert_eq!(partial.free_cash_flow_ttm, None);
    assert_eq!(partial.gross_margin_percent, None, "營收為 0 不算毛利率");

    assert_eq!(
        snapshot("0050".to_owned(), &BTreeMap::new(), &[]),
        FinancialHealthSnapshot {
            stock_symbol: "0050".to_owned(),
            f_score_year: None,
            f_score: None,
            f_score_evaluated: 0,
            quarter_year: None,
            quarter: None,
            gross_margin_percent: None,
            debt_ratio_percent: None,
            free_cash_flow_ttm: None,
        }
    );
}

#[test]
fn percent_rounds_to_four_decimals() {
    assert_eq!(percent(Some(1), Some(3)), Some(dec!(33.3333)));
    assert_eq!(percent(Some(-5), Some(4)), Some(dec!(-125)));
    assert_eq!(percent(Some(1), Some(0)), None);
    assert_eq!(percent(None, Some(1)), None);
}
