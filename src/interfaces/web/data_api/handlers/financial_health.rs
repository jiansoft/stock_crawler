//! `/stocks/{symbol}/financial-health` handler：以年度三大報表計算 Piotroski F 分數。
//!
//! 9 項指標比較評分年度與前一年度，每項通過得 1 分：
//!
//! | 分類 | 指標 | 得分條件 |
//! |---|---|---|
//! | 獲利能力 | 資產報酬率 | 稅後淨利 ÷ 總資產 > 0 |
//! | | 營業現金流 | > 0 |
//! | | 資產報酬率變化 | 高於前一年 |
//! | | 盈餘品質 | 營業現金流 > 稅後淨利 |
//! | 槓桿與流動性 | 長期負債比 | 非流動負債 ÷ 總資產，不高於前一年 |
//! | | 流動比率 | 高於前一年 |
//! | | 股本 | 不高於前一年（盈餘轉增資也算增加） |
//! | 經營效率 | 毛利率 | 高於前一年 |
//! | | 資產周轉率 | 營收 ÷ 總資產，高於前一年 |
//!
//! 原論文的比率以年初總資產為分母；這裡一律用當年度期末數值，只需要兩個年度的資料，
//! 多數股票都算得出來。損益與現金流量用年度報表，資產負債表取各年第 4 季季末；
//! 同一期 Yahoo 與 BigGo 並存時取 Yahoo。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{database_error, ensure_stock_exists, error_response};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, FinancialHealthParams, FinancialHealthResponse, HEALTH_MAX_SCORE, HealthCriterion,
};

/// 單一年度計算 F 分數用到的數值（新台幣千元）。
#[derive(Debug, Default, Clone, Copy, PartialEq, sqlx::FromRow)]
struct YearFigures {
    revenue: Option<i64>,
    gross_profit: Option<i64>,
    net_income: Option<i64>,
    operating_cash_flow: Option<i64>,
    total_assets: Option<i64>,
    current_assets: Option<i64>,
    current_liabilities: Option<i64>,
    non_current_liabilities: Option<i64>,
    share_capital: Option<i64>,
}

impl YearFigures {
    fn roa(&self) -> Option<f64> {
        ratio(self.net_income, self.total_assets)
    }

    /// 長期負債比；流動與非流動負債都是 0 代表來源沒有負債分類（金融業），不適用。
    fn long_term_debt_ratio(&self) -> Option<f64> {
        let current = self.current_liabilities?;
        let non_current = self.non_current_liabilities?;
        if current == 0 && non_current == 0 {
            return None;
        }
        ratio(Some(non_current), self.total_assets)
    }

    fn current_ratio(&self) -> Option<f64> {
        ratio(self.current_assets, self.current_liabilities)
    }

    fn gross_margin(&self) -> Option<f64> {
        ratio(self.gross_profit, self.revenue)
    }

    fn asset_turnover(&self) -> Option<f64> {
        ratio(self.revenue, self.total_assets)
    }
}

/// 兩個數值都有、分母不為 0 時才算得出比率。
fn ratio(numerator: Option<i64>, denominator: Option<i64>) -> Option<f64> {
    match (numerator, denominator) {
        (Some(numerator), Some(denominator)) if denominator != 0 => {
            Some(numerator as f64 / denominator as f64)
        }
        _ => None,
    }
}

fn amount(value: Option<i64>) -> Option<f64> {
    value.map(|value| value as f64)
}

/// 比率取到小數第 4 位（0.01%）；是否得分用未四捨五入的值判斷。
fn round_ratio(value: Option<f64>) -> Option<f64> {
    value.map(|value| (value * 10_000.0).round() / 10_000.0)
}

const PROFITABILITY: &str = "profitability";
const LEVERAGE_LIQUIDITY: &str = "leverage_liquidity";
const OPERATING_EFFICIENCY: &str = "operating_efficiency";
const RATIO: &str = "ratio";
const THOUSAND_TWD: &str = "thousand_twd";

/// 組成一項指標；`value` 與 `baseline` 都有值時才以 `rule` 判斷是否得分。
fn criterion(
    (key, name, category): (&'static str, &'static str, &'static str),
    unit: &'static str,
    value: Option<f64>,
    baseline: Option<f64>,
    rule: fn(f64, f64) -> bool,
) -> HealthCriterion {
    let passed = value
        .zip(baseline)
        .map(|(value, baseline)| rule(value, baseline));
    let (value, baseline) = if unit == RATIO {
        (round_ratio(value), round_ratio(baseline))
    } else {
        (value, baseline)
    };
    HealthCriterion {
        key,
        name,
        category,
        passed,
        value,
        baseline,
        unit,
    }
}

fn greater(value: f64, baseline: f64) -> bool {
    value > baseline
}

fn not_greater(value: f64, baseline: f64) -> bool {
    value <= baseline
}

/// 依 Piotroski 的 9 項指標評分，順序固定。
fn piotroski(current: &YearFigures, previous: &YearFigures) -> Vec<HealthCriterion> {
    vec![
        criterion(
            ("roa_positive", "資產報酬率為正", PROFITABILITY),
            RATIO,
            current.roa(),
            Some(0.0),
            greater,
        ),
        criterion(
            (
                "operating_cash_flow_positive",
                "營業現金流為正",
                PROFITABILITY,
            ),
            THOUSAND_TWD,
            amount(current.operating_cash_flow),
            Some(0.0),
            greater,
        ),
        criterion(
            ("roa_improved", "資產報酬率較前一年提高", PROFITABILITY),
            RATIO,
            current.roa(),
            previous.roa(),
            greater,
        ),
        criterion(
            (
                "cash_flow_exceeds_net_income",
                "營業現金流高於稅後淨利",
                PROFITABILITY,
            ),
            THOUSAND_TWD,
            amount(current.operating_cash_flow),
            amount(current.net_income),
            greater,
        ),
        criterion(
            (
                "leverage_not_increased",
                "長期負債比未較前一年上升",
                LEVERAGE_LIQUIDITY,
            ),
            RATIO,
            current.long_term_debt_ratio(),
            previous.long_term_debt_ratio(),
            not_greater,
        ),
        criterion(
            (
                "current_ratio_improved",
                "流動比率較前一年提高",
                LEVERAGE_LIQUIDITY,
            ),
            RATIO,
            current.current_ratio(),
            previous.current_ratio(),
            greater,
        ),
        criterion(
            ("no_new_shares", "股本未較前一年增加", LEVERAGE_LIQUIDITY),
            THOUSAND_TWD,
            amount(current.share_capital),
            amount(previous.share_capital),
            not_greater,
        ),
        criterion(
            (
                "gross_margin_improved",
                "毛利率較前一年提高",
                OPERATING_EFFICIENCY,
            ),
            RATIO,
            current.gross_margin(),
            previous.gross_margin(),
            greater,
        ),
        criterion(
            (
                "asset_turnover_improved",
                "資產周轉率較前一年提高",
                OPERATING_EFFICIENCY,
            ),
            RATIO,
            current.asset_turnover(),
            previous.asset_turnover(),
            greater,
        ),
    ]
}

/// 得分與可判定的指標數。
fn tally(criteria: &[HealthCriterion]) -> (u8, u8) {
    let count = |passed: fn(&Option<bool>) -> bool| {
        u8::try_from(criteria.iter().filter(|c| passed(&c.passed)).count()).unwrap_or(u8::MAX)
    };
    (count(|p| *p == Some(true)), count(Option::is_some))
}

/// 以 Piotroski F 分數評估單一股票的年度財務體質。
///
/// 回傳 9 項指標各自的本期數值、比較基準與是否得分；資料不足或不適用的指標
/// 為 `null`、不計分，`evaluated` 是可判定的指標數。
///
/// # Errors
///
/// `fiscal_year` 不在 2000–2100 回 422、股票不存在回 404、驗證失敗回 401；
/// 資料庫查詢失敗時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/financial-health", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), FinancialHealthParams), responses((status = 200, body = FinancialHealthResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn financial_health(
    Path(symbol): Path<String>,
    Query(params): Query<FinancialHealthParams>,
) -> Response {
    if params
        .fiscal_year
        .is_some_and(|year| !(2000..=2100).contains(&year))
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "fiscal_year 必須介於 2000 至 2100",
        );
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    let pool = database::get_connection();

    let fiscal_year = match params.fiscal_year {
        Some(year) => Some(year),
        None => match sqlx::query_scalar::<_, Option<i32>>(
            "SELECT max(fiscal_year) FROM income_statement WHERE stock_symbol = $1 AND period_type = 'annual'",
        )
        .bind(&symbol)
        .fetch_one(pool)
        .await
        {
            Ok(year) => year,
            Err(error) => return database_error(error),
        },
    };

    let (current, previous) = match fiscal_year {
        Some(year) => {
            // 兩個年度各取一列：三張表都用 DISTINCT ON 讓同一年只留一列、Yahoo 優先；
            // 沒有資料的年度由 VALUES 補出全 NULL 的列。
            let rows: Vec<(i32, YearFigures)> = match sqlx::query_as::<_, YearRow>(
                r#"WITH years(fiscal_year) AS (VALUES ($2::int), ($2::int - 1)),
inc AS (SELECT DISTINCT ON (fiscal_year) fiscal_year, revenue, gross_profit, net_income FROM income_statement WHERE stock_symbol = $1 AND period_type = 'annual' AND fiscal_year IN ($2::int, $2::int - 1) ORDER BY fiscal_year, (source = 'yahoo') DESC, source),
cf AS (SELECT DISTINCT ON (fiscal_year) fiscal_year, operating_cash_flow FROM cash_flow_statement WHERE stock_symbol = $1 AND period_type = 'annual' AND fiscal_year IN ($2::int, $2::int - 1) ORDER BY fiscal_year, (source = 'yahoo') DESC, source),
bs AS (SELECT DISTINCT ON (fiscal_year) fiscal_year, total_assets, current_assets, current_liabilities, non_current_liabilities, share_capital FROM balance_sheet WHERE stock_symbol = $1 AND period_type = 'single' AND quarter = 'Q4' AND fiscal_year IN ($2::int, $2::int - 1) ORDER BY fiscal_year, (source = 'yahoo') DESC, source)
SELECT years.fiscal_year, inc.revenue, inc.gross_profit, inc.net_income, cf.operating_cash_flow, bs.total_assets, bs.current_assets, bs.current_liabilities, bs.non_current_liabilities, bs.share_capital
FROM years LEFT JOIN inc USING (fiscal_year) LEFT JOIN cf USING (fiscal_year) LEFT JOIN bs USING (fiscal_year)"#,
            )
            .bind(&symbol)
            .bind(year)
            .fetch_all(pool)
            .await
            {
                Ok(rows) => rows
                    .into_iter()
                    .map(|row| (row.fiscal_year, row.figures))
                    .collect(),
                Err(error) => return database_error(error),
            };
            let figures = |target: i32| {
                rows.iter()
                    .find(|(row_year, _)| *row_year == target)
                    .map(|(_, figures)| *figures)
                    .unwrap_or_default()
            };
            (figures(year), figures(year - 1))
        }
        None => (YearFigures::default(), YearFigures::default()),
    };

    let criteria = piotroski(&current, &previous);
    let (score, evaluated) = tally(&criteria);
    Json(FinancialHealthResponse {
        stock_symbol: symbol,
        fiscal_year,
        compared_with: fiscal_year.map(|year| year - 1),
        score,
        evaluated,
        max_score: HEALTH_MAX_SCORE,
        criteria,
    })
    .into_response()
}

/// SQL 回傳的一列：年度加上該年度的數值。
#[derive(sqlx::FromRow)]
struct YearRow {
    fiscal_year: i32,
    #[sqlx(flatten)]
    figures: YearFigures,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一般公司兩個年度的數值：除了長期負債比上升，其餘 8 項都改善。
    fn healthy_pair() -> (YearFigures, YearFigures) {
        let current = YearFigures {
            revenue: Some(4_600),
            gross_profit: Some(1_900),
            net_income: Some(460),
            operating_cash_flow: Some(900),
            total_assets: Some(52_000),
            current_assets: Some(20_000),
            current_liabilities: Some(10_000),
            non_current_liabilities: Some(11_000),
            share_capital: Some(1_000),
        };
        let previous = YearFigures {
            revenue: Some(4_000),
            gross_profit: Some(1_500),
            net_income: Some(300),
            operating_cash_flow: Some(200),
            total_assets: Some(48_000),
            current_assets: Some(15_000),
            current_liabilities: Some(9_000),
            non_current_liabilities: Some(10_000),
            share_capital: Some(1_000),
        };
        (current, previous)
    }

    fn passed(criteria: &[HealthCriterion]) -> Vec<(&'static str, Option<bool>)> {
        criteria.iter().map(|c| (c.key, c.passed)).collect()
    }

    #[test]
    fn piotroski_scores_each_criterion_in_fixed_order() {
        let (current, previous) = healthy_pair();
        let criteria = piotroski(&current, &previous);
        assert_eq!(
            passed(&criteria),
            vec![
                ("roa_positive", Some(true)),
                ("operating_cash_flow_positive", Some(true)),
                ("roa_improved", Some(true)),
                ("cash_flow_exceeds_net_income", Some(true)),
                ("leverage_not_increased", Some(false)),
                ("current_ratio_improved", Some(true)),
                ("no_new_shares", Some(true)),
                ("gross_margin_improved", Some(true)),
                ("asset_turnover_improved", Some(true)),
            ]
        );
        assert_eq!(tally(&criteria), (8, 9));

        // 比率取 4 位小數、金額原樣；分類與單位照表。
        let roa = &criteria[0];
        assert_eq!((roa.value, roa.baseline), (Some(0.0088), Some(0.0)));
        assert_eq!((roa.category, roa.unit), ("profitability", "ratio"));
        let accrual = &criteria[3];
        assert_eq!(
            (accrual.value, accrual.baseline),
            (Some(900.0), Some(460.0))
        );
        assert_eq!(accrual.unit, "thousand_twd");
        let leverage = &criteria[4];
        assert_eq!(
            (leverage.value, leverage.baseline),
            (Some(0.2115), Some(0.2083))
        );
        assert_eq!(leverage.category, "leverage_liquidity");
        assert_eq!(criteria[8].category, "operating_efficiency");
    }

    /// 長期負債比與股本「持平」算得分；其餘「提高」類指標持平不得分。
    #[test]
    fn piotroski_treats_unchanged_leverage_and_shares_as_passing() {
        let (current, _) = healthy_pair();
        let criteria = piotroski(&current, &current);
        let by_key: Vec<_> = passed(&criteria);
        assert_eq!(by_key[2], ("roa_improved", Some(false)));
        assert_eq!(by_key[4], ("leverage_not_increased", Some(true)));
        assert_eq!(by_key[6], ("no_new_shares", Some(true)));
        assert_eq!(by_key[7], ("gross_margin_improved", Some(false)));
    }

    /// 金融業的流動／非流動負債都是 0：長期負債比與流動比率不適用，不算分也不算失分。
    #[test]
    fn piotroski_skips_liquidity_criteria_without_liability_breakdown() {
        let (mut current, mut previous) = healthy_pair();
        for figures in [&mut current, &mut previous] {
            figures.current_assets = Some(0);
            figures.current_liabilities = Some(0);
            figures.non_current_liabilities = Some(0);
        }
        let criteria = piotroski(&current, &previous);
        assert_eq!(criteria[4].passed, None);
        assert_eq!(criteria[4].value, None);
        assert_eq!(criteria[5].passed, None);
        assert_eq!(tally(&criteria), (7, 7));
    }

    /// 沒有前一年度：只剩不需比較的 3 項可判定。
    #[test]
    fn piotroski_without_previous_year_evaluates_three_criteria() {
        let (current, _) = healthy_pair();
        let criteria = piotroski(&current, &YearFigures::default());
        let evaluated: Vec<_> = criteria
            .iter()
            .filter(|c| c.passed.is_some())
            .map(|c| c.key)
            .collect();
        assert_eq!(
            evaluated,
            [
                "roa_positive",
                "operating_cash_flow_positive",
                "cash_flow_exceeds_net_income"
            ]
        );
        assert_eq!(
            tally(&piotroski(&YearFigures::default(), &YearFigures::default())),
            (0, 0)
        );
    }

    #[test]
    fn ratio_requires_both_values_and_non_zero_denominator() {
        assert_eq!(ratio(Some(1), Some(4)), Some(0.25));
        assert_eq!(ratio(Some(1), Some(0)), None);
        assert_eq!(ratio(None, Some(4)), None);
        assert_eq!(ratio(Some(1), None), None);
        assert_eq!(round_ratio(Some(0.123_456)), Some(0.1235));
    }
}
