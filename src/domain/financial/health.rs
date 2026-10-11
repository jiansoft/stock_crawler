//! 財務體質評分：年度 Piotroski F 分數，以及條件選股用的最新一季指標。
//!
//! 純計算、不做 I/O：單檔查詢（Data API）與每日全市場重算共用同一套規則。
//!
//! F 分數的 9 項指標比較評分年度與前一年度，每項通過得 1 分：
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
//! 多數股票都算得出來。

use std::collections::BTreeMap;

use rust_decimal::Decimal;

/// F 分數的滿分。
pub const MAX_SCORE: u8 = 9;

/// 單項 F 分數指標。
#[derive(Debug, Clone, PartialEq)]
pub struct HealthCriterion {
    /// 指標代碼，例如 `roa_positive`。
    pub key: &'static str,
    /// 指標名稱。
    pub name: &'static str,
    /// 分類：`profitability`、`leverage_liquidity` 或 `operating_efficiency`。
    pub category: &'static str,
    /// 是否得分；資料不足或不適用為 `None`。
    pub passed: Option<bool>,
    /// 評分年度的數值。
    pub value: Option<f64>,
    /// 比較基準。
    pub baseline: Option<f64>,
    /// 數值單位：`ratio` 或 `thousand_twd`。
    pub unit: &'static str,
}

/// 單一年度計算 F 分數用到的數值（新台幣千元）。
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct YearFigures {
    pub revenue: Option<i64>,
    pub gross_profit: Option<i64>,
    pub net_income: Option<i64>,
    pub operating_cash_flow: Option<i64>,
    pub total_assets: Option<i64>,
    pub current_assets: Option<i64>,
    pub current_liabilities: Option<i64>,
    pub non_current_liabilities: Option<i64>,
    pub share_capital: Option<i64>,
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

pub const PROFITABILITY: &str = "profitability";
pub const LEVERAGE_LIQUIDITY: &str = "leverage_liquidity";
pub const OPERATING_EFFICIENCY: &str = "operating_efficiency";
pub const RATIO: &str = "ratio";
pub const THOUSAND_TWD: &str = "thousand_twd";

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
pub fn piotroski(current: &YearFigures, previous: &YearFigures) -> Vec<HealthCriterion> {
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
pub fn tally(criteria: &[HealthCriterion]) -> (u8, u8) {
    let count = |passed: fn(&Option<bool>) -> bool| {
        u8::try_from(criteria.iter().filter(|c| passed(&c.passed)).count()).unwrap_or(u8::MAX)
    };
    (count(|p| *p == Some(true)), count(Option::is_some))
}

/// 單一季度的選股用數值（新台幣千元）；`has_income` 表示這一季有損益表。
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct QuarterFigures {
    pub fiscal_year: i32,
    /// 1–4。
    pub quarter: u8,
    pub has_income: bool,
    pub revenue: Option<i64>,
    pub gross_profit: Option<i64>,
    pub total_assets: Option<i64>,
    pub total_liabilities: Option<i64>,
    pub free_cash_flow: Option<i64>,
}

impl QuarterFigures {
    /// 以「年 × 4 + 季」排序與判斷連續季度。
    fn ordinal(&self) -> i32 {
        self.fiscal_year * 4 + i32::from(self.quarter)
    }
}

/// 單一年度的年報數值；`has_income` 表示這一年有年度損益表，F 分數以此決定評分年度。
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct AnnualFigures {
    pub has_income: bool,
    pub figures: YearFigures,
}

/// 每日全市場重算後，一檔股票的財務體質。
#[derive(Debug, Clone, PartialEq)]
pub struct FinancialHealthSnapshot {
    pub stock_symbol: String,
    /// F 分數的評分年度：最新一個有年度損益表的年度。
    pub f_score_year: Option<i32>,
    /// F 分數；沒有評分年度時為 `None`。
    pub f_score: Option<u8>,
    /// 可判定的指標數。
    pub f_score_evaluated: u8,
    /// 季報指標的期間：最新一個有損益表的季度。
    pub quarter_year: Option<i32>,
    pub quarter: Option<u8>,
    /// 該季毛利率（%）。
    pub gross_margin_percent: Option<Decimal>,
    /// 該季季末負債比（%）。
    pub debt_ratio_percent: Option<Decimal>,
    /// 截至該季的近四季自由現金流合計（千元）；四季缺任一季為 `None`。
    pub free_cash_flow_ttm: Option<i64>,
}

/// 由一檔股票的年報與季報數值算出財務體質。
///
/// `annual` 以年度為 key；`quarters` 順序不拘。
pub fn snapshot(
    stock_symbol: String,
    annual: &BTreeMap<i32, AnnualFigures>,
    quarters: &[QuarterFigures],
) -> FinancialHealthSnapshot {
    let f_score_year = annual
        .iter()
        .rev()
        .find(|(_, figures)| figures.has_income)
        .map(|(year, _)| *year);
    let (f_score, f_score_evaluated) = match f_score_year {
        Some(year) => {
            let figures = |year: i32| annual.get(&year).map(|a| a.figures).unwrap_or_default();
            let (score, evaluated) = tally(&piotroski(&figures(year), &figures(year - 1)));
            (Some(score), evaluated)
        }
        None => (None, 0),
    };

    let latest = quarters
        .iter()
        .filter(|q| q.has_income)
        .max_by_key(|q| q.ordinal());
    let free_cash_flow_ttm = latest.and_then(|latest| {
        (0..4)
            .map(|offset| {
                quarters
                    .iter()
                    .find(|q| q.ordinal() == latest.ordinal() - offset)
                    .and_then(|q| q.free_cash_flow)
            })
            .sum::<Option<i64>>()
    });

    FinancialHealthSnapshot {
        stock_symbol,
        f_score_year,
        f_score,
        f_score_evaluated,
        quarter_year: latest.map(|q| q.fiscal_year),
        quarter: latest.map(|q| q.quarter),
        gross_margin_percent: latest.and_then(|q| percent(q.gross_profit, q.revenue)),
        debt_ratio_percent: latest.and_then(|q| percent(q.total_liabilities, q.total_assets)),
        free_cash_flow_ttm,
    }
}

/// 百分比取到小數第 4 位；任一缺值或分母為 0 時為 `None`。
fn percent(numerator: Option<i64>, denominator: Option<i64>) -> Option<Decimal> {
    match (numerator, denominator) {
        (Some(numerator), Some(denominator)) if denominator != 0 => Some(
            (Decimal::from(numerator) * Decimal::ONE_HUNDRED / Decimal::from(denominator))
                .round_dp(4),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

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
}
