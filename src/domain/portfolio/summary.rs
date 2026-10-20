//! # 持股組合總覽
//!
//! 把目前持有（未賣出）的各檔部位彙總成一份總覽：市值與權重、未實現損益、
//! 已領股利、產業分布，以及以市值加權的殖利率、Piotroski F 分數、本益比與股價淨值比。
//!
//! - 市值＝持有股數 × 最新收盤價；沒有收盤價的部位不計市值、權重與損益，
//!   但股數、成本與已領股利照常列出。
//! - 加權殖利率與加權 F 分數只用「有該指標」的部位加權，另回報這些部位占總市值的
//!   比例（覆蓋率），避免少數有值的部位被誤當成整體。
//! - 本益比與股價淨值比用市值加權的調和平均（Σ市值 ÷ Σ(市值 ÷ 比值)），等同把整個組合當成一家公司：
//!   總市值 ÷ 依持股比例分得的盈餘（或淨值）。虧損（本益比 0）的部位不列入。
//! - 成本是買進時付出的金額（含手續費），配股取得的股數成本為 0。

use std::collections::HashMap;

use rust_decimal::{Decimal, prelude::ToPrimitive};

/// 單一股票的持有部位（同代號的多筆買進已合併）。
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 產業分類編號；股票主檔找不到時為 `None`。
    pub industry_id: Option<i32>,
    /// 產業名稱；找不到時為空字串。
    pub industry_name: String,
    /// 持有股數。
    pub shares: i64,
    /// 持有成本（元，正數）。
    pub cost: Decimal,
    /// 這些部位累計已領股利（元）。
    pub dividends_received: Decimal,
    /// 最新收盤價；沒有報價或收盤價為 0 時為 `None`。
    pub closing_price: Option<Decimal>,
    /// 最新殖利率（%）；沒有或過期時為 `None`。
    pub dividend_yield_percent: Option<Decimal>,
    /// Piotroski F 分數；沒有或過期時為 `None`。
    pub f_score: Option<i16>,
    /// 最新一筆日線的官方本益比；虧損或沒有資料時為 `None`。
    pub pe_ratio: Option<Decimal>,
    /// 最新一筆日線的官方股價淨值比；沒有資料時為 `None`。
    pub pb_ratio: Option<Decimal>,
}

/// 部位加上市值、權重與損益後的結果。
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingSummary {
    /// 原始部位。
    pub position: Position,
    /// 市值（元）；沒有收盤價時為 `None`。
    pub market_value: Option<f64>,
    /// 占總市值的百分比；沒有收盤價時為 `None`。
    pub weight_percent: Option<f64>,
    /// 未實現損益（市值 − 成本）；沒有收盤價時為 `None`。
    pub unrealized_gain: Option<f64>,
    /// 未實現報酬率（%）；沒有收盤價或成本為 0（全為配股）時為 `None`。
    pub unrealized_gain_percent: Option<f64>,
}

/// 單一產業的彙總。
#[derive(Debug, Clone, PartialEq)]
pub struct IndustrySummary {
    /// 產業分類編號。
    pub industry_id: Option<i32>,
    /// 產業名稱。
    pub industry_name: String,
    /// 該產業持股的市值合計。
    pub market_value: f64,
    /// 占總市值的百分比。
    pub weight_percent: f64,
    /// 持有幾檔。
    pub symbols: usize,
}

/// 以市值加權的指標與其覆蓋率。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightedMetric {
    /// 加權平均值。
    pub value: f64,
    /// 有此指標的部位占總市值的百分比。
    pub coverage_percent: f64,
}

/// 持股組合總覽。
#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioSummary {
    /// 持有檔數。
    pub symbols: usize,
    /// 總市值（只計有收盤價的部位）。
    pub market_value: f64,
    /// 總成本（所有部位）。
    pub cost: f64,
    /// 有收盤價部位的未實現損益合計。
    pub unrealized_gain: f64,
    /// 未實現報酬率（%），分母只含有收盤價部位的成本；該成本為 0 時為 `None`。
    pub unrealized_gain_percent: Option<f64>,
    /// 目前持股累計已領股利。
    pub dividends_received: f64,
    /// 市值加權殖利率；沒有任何部位有殖利率時為 `None`。
    pub weighted_dividend_yield: Option<WeightedMetric>,
    /// 市值加權 F 分數；沒有任何部位有 F 分數時為 `None`。
    pub weighted_f_score: Option<WeightedMetric>,
    /// 市值加權（調和平均）本益比；沒有任何部位有本益比時為 `None`。
    pub weighted_pe: Option<WeightedMetric>,
    /// 市值加權（調和平均）股價淨值比；沒有任何部位有淨值比時為 `None`。
    pub weighted_pb: Option<WeightedMetric>,
    /// 產業分布，依市值由大到小。
    pub industries: Vec<IndustrySummary>,
    /// 各部位，依市值由大到小（沒有市值的排最後、再依代號）。
    pub holdings: Vec<HoldingSummary>,
}

/// 由各部位計算持股組合總覽。
pub fn summarize(positions: Vec<Position>) -> PortfolioSummary {
    let market_values: Vec<Option<Decimal>> = positions
        .iter()
        .map(|position| {
            position
                .closing_price
                .filter(|price| *price > Decimal::ZERO)
                .map(|price| price * Decimal::from(position.shares))
        })
        .collect();
    let total_value: Decimal = market_values.iter().flatten().sum();
    let total_cost: Decimal = positions.iter().map(|position| position.cost).sum();
    let priced_cost: Decimal = positions
        .iter()
        .zip(&market_values)
        .filter(|(_, value)| value.is_some())
        .map(|(position, _)| position.cost)
        .sum();
    let dividends: Decimal = positions
        .iter()
        .map(|position| position.dividends_received)
        .sum();
    let weight = |value: Decimal| percent(value, total_value);

    let weighted_dividend_yield = weighted(&positions, &market_values, total_value, |position| {
        position.dividend_yield_percent
    });
    let weighted_f_score = weighted(&positions, &market_values, total_value, |position| {
        position.f_score.map(Decimal::from)
    });
    let weighted_pe = weighted_harmonic(&positions, &market_values, total_value, |position| {
        position.pe_ratio
    });
    let weighted_pb = weighted_harmonic(&positions, &market_values, total_value, |position| {
        position.pb_ratio
    });

    let mut by_industry: HashMap<Option<i32>, (String, Decimal, usize)> = HashMap::new();
    for (position, value) in positions.iter().zip(&market_values) {
        let entry = by_industry
            .entry(position.industry_id)
            .or_insert_with(|| (position.industry_name.clone(), Decimal::ZERO, 0));
        entry.1 += value.unwrap_or_default();
        entry.2 += 1;
    }
    let mut industries: Vec<IndustrySummary> = by_industry
        .into_iter()
        .map(
            |(industry_id, (industry_name, value, symbols))| IndustrySummary {
                industry_id,
                industry_name,
                market_value: money(value),
                weight_percent: weight(value).unwrap_or(0.0),
                symbols,
            },
        )
        .collect();
    industries.sort_by(|a, b| {
        b.market_value
            .total_cmp(&a.market_value)
            .then_with(|| a.industry_id.cmp(&b.industry_id))
    });

    let mut holdings: Vec<HoldingSummary> = positions
        .into_iter()
        .zip(market_values)
        .map(|(position, value)| {
            let gain = value.map(|value| value - position.cost);
            HoldingSummary {
                market_value: value.map(money),
                weight_percent: value.and_then(weight),
                unrealized_gain: gain.map(money),
                unrealized_gain_percent: gain.and_then(|gain| percent(gain, position.cost)),
                position,
            }
        })
        .collect();
    holdings.sort_by(|a, b| {
        b.market_value
            .unwrap_or(f64::MIN)
            .total_cmp(&a.market_value.unwrap_or(f64::MIN))
            .then_with(|| a.position.stock_symbol.cmp(&b.position.stock_symbol))
    });

    let unrealized_gain = total_value - priced_cost;
    PortfolioSummary {
        symbols: holdings.len(),
        market_value: money(total_value),
        cost: money(total_cost),
        unrealized_gain: money(unrealized_gain),
        unrealized_gain_percent: percent(unrealized_gain, priced_cost),
        dividends_received: money(dividends),
        weighted_dividend_yield,
        weighted_f_score,
        weighted_pe,
        weighted_pb,
        industries,
        holdings,
    }
}

/// 以市值加權 `metric` 的調和平均（Σ市值 ÷ Σ(市值 ÷ 比值)）；只有比值為正且有市值的部位參與。
fn weighted_harmonic(
    positions: &[Position],
    market_values: &[Option<Decimal>],
    total_value: Decimal,
    metric: impl Fn(&Position) -> Option<Decimal>,
) -> Option<WeightedMetric> {
    let (inverse_sum, covered) = positions.iter().zip(market_values).fold(
        (Decimal::ZERO, Decimal::ZERO),
        |(sum, covered), (position, value)| match (metric(position), value) {
            (Some(metric), Some(value)) if metric > Decimal::ZERO => {
                (sum + *value / metric, covered + value)
            }
            _ => (sum, covered),
        },
    );
    if inverse_sum.is_zero() {
        return None;
    }
    Some(WeightedMetric {
        value: round_to(covered / inverse_sum, 2),
        coverage_percent: percent(covered, total_value).unwrap_or(0.0),
    })
}

/// 以市值加權 `metric`；只有有值且有市值的部位參與。
fn weighted(
    positions: &[Position],
    market_values: &[Option<Decimal>],
    total_value: Decimal,
    metric: impl Fn(&Position) -> Option<Decimal>,
) -> Option<WeightedMetric> {
    let (weighted_sum, covered) = positions.iter().zip(market_values).fold(
        (Decimal::ZERO, Decimal::ZERO),
        |(sum, covered), (position, value)| match (metric(position), value) {
            (Some(metric), Some(value)) => (sum + metric * value, covered + value),
            _ => (sum, covered),
        },
    );
    if covered.is_zero() {
        return None;
    }
    Some(WeightedMetric {
        value: round_to(weighted_sum / covered, 2),
        coverage_percent: percent(covered, total_value).unwrap_or(0.0),
    })
}

/// `part / whole` 的百分比（小數兩位）；分母為 0 時為 `None`。
fn percent(part: Decimal, whole: Decimal) -> Option<f64> {
    (!whole.is_zero()).then(|| round_to(part / whole * Decimal::ONE_HUNDRED, 2))
}

/// 金額四捨五入到小數兩位。
fn money(value: Decimal) -> f64 {
    round_to(value, 2)
}

fn round_to(value: Decimal, digits: u32) -> f64 {
    value.round_dp(digits).to_f64().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn position(symbol: &str, industry: Option<i32>, shares: i64, cost: Decimal) -> Position {
        Position {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol} 名稱"),
            industry_id: industry,
            industry_name: industry.map(|id| format!("產業{id}")).unwrap_or_default(),
            shares,
            cost,
            dividends_received: Decimal::ZERO,
            closing_price: None,
            dividend_yield_percent: None,
            f_score: None,
            pe_ratio: None,
            pb_ratio: None,
        }
    }

    /// 本益比與股價淨值比用市值加權的調和平均；非正值（虧損）不列入，也不算覆蓋。
    #[test]
    fn summarize_uses_harmonic_mean_for_valuation_ratios() {
        let positions = vec![
            Position {
                closing_price: Some(dec!(100)),
                pe_ratio: Some(dec!(10)),
                pb_ratio: Some(dec!(1)),
                ..position("1101", Some(1), 1_000, dec!(80_000))
            },
            Position {
                closing_price: Some(dec!(100)),
                pe_ratio: Some(dec!(40)),
                pb_ratio: Some(dec!(4)),
                ..position("2330", Some(24), 1_000, dec!(60_000))
            },
            Position {
                closing_price: Some(dec!(100)),
                pe_ratio: Some(Decimal::ZERO),
                ..position("2603", Some(15), 1_000, dec!(90_000))
            },
        ];
        let summary = summarize(positions);
        // 200,000 ÷ (100,000/10 + 100,000/40)＝16；覆蓋 2/3 市值。
        assert_eq!(
            summary.weighted_pe,
            Some(WeightedMetric {
                value: 16.0,
                coverage_percent: 66.67
            })
        );
        assert_eq!(summary.weighted_pb.map(|metric| metric.value), Some(1.6));
    }

    /// 市值、權重、損益、產業分布與市值加權指標。
    #[test]
    fn summarize_weights_by_market_value() {
        let positions = vec![
            Position {
                closing_price: Some(dec!(100)),
                dividend_yield_percent: Some(dec!(4)),
                f_score: Some(8),
                dividends_received: dec!(500),
                ..position("1101", Some(1), 1_000, dec!(80_000))
            },
            Position {
                closing_price: Some(dec!(50)),
                dividend_yield_percent: Some(dec!(2)),
                ..position("2330", Some(24), 1_000, dec!(60_000))
            },
            Position {
                closing_price: Some(dec!(25)),
                f_score: Some(5),
                ..position("1102", Some(1), 2_000, dec!(40_000))
            },
        ];
        let summary = summarize(positions);

        // 市值 100,000＋50,000＋50,000＝200,000；成本 180,000。
        assert_eq!((summary.market_value, summary.cost), (200_000.0, 180_000.0));
        assert_eq!(summary.unrealized_gain, 20_000.0);
        assert_eq!(summary.unrealized_gain_percent, Some(11.11));
        assert_eq!(summary.dividends_received, 500.0);
        // 殖利率只看前兩檔：(4×100k＋2×50k)/150k＝3.33，覆蓋 75%。
        assert_eq!(
            summary.weighted_dividend_yield,
            Some(WeightedMetric {
                value: 3.33,
                coverage_percent: 75.0
            })
        );
        // F 分數：(8×100k＋5×50k)/150k＝7。
        assert_eq!(
            summary.weighted_f_score.map(|metric| metric.value),
            Some(7.0)
        );

        assert_eq!(summary.industries.len(), 2);
        assert_eq!(summary.industries[0].industry_id, Some(1));
        assert_eq!(
            (
                summary.industries[0].weight_percent,
                summary.industries[0].symbols
            ),
            (75.0, 2)
        );
        assert_eq!(summary.industries[1].weight_percent, 25.0);

        let first = &summary.holdings[0];
        assert_eq!(first.position.stock_symbol, "1101");
        assert_eq!(
            (
                first.weight_percent,
                first.unrealized_gain,
                first.unrealized_gain_percent
            ),
            (Some(50.0), Some(20_000.0), Some(25.0))
        );
        // 同市值依代號排序。
        assert_eq!(summary.holdings[1].position.stock_symbol, "1102");
        assert_eq!(summary.holdings[1].unrealized_gain_percent, Some(25.0));
        assert_eq!(summary.holdings[2].unrealized_gain_percent, Some(-16.67));
    }

    /// 沒有收盤價的部位不計市值與損益；全為配股（成本 0）沒有報酬率；沒有任何指標時加權為 null。
    #[test]
    fn summarize_handles_missing_price_and_zero_cost() {
        let positions = vec![
            position("9999", None, 500, dec!(10_000)),
            Position {
                closing_price: Some(dec!(10)),
                ..position("2887", Some(17), 67, Decimal::ZERO)
            },
        ];
        let summary = summarize(positions);
        assert_eq!((summary.market_value, summary.cost), (670.0, 10_000.0));
        // 損益只算有報價的部位：670 − 0，成本 0 → 沒有報酬率。
        assert_eq!(summary.unrealized_gain, 670.0);
        assert_eq!(summary.unrealized_gain_percent, None);
        assert_eq!(summary.weighted_dividend_yield, None);
        assert_eq!(summary.weighted_f_score, None);
        assert_eq!((summary.weighted_pe, summary.weighted_pb), (None, None));

        let priced = &summary.holdings[0];
        assert_eq!(priced.position.stock_symbol, "2887");
        assert_eq!(priced.unrealized_gain_percent, None);
        let unpriced = &summary.holdings[1];
        assert_eq!(
            (
                unpriced.market_value,
                unpriced.weight_percent,
                unpriced.unrealized_gain
            ),
            (None, None, None)
        );
        let unknown_industry = summary
            .industries
            .iter()
            .find(|industry| industry.industry_id.is_none())
            .expect("未知產業也要列出");
        assert_eq!(
            (
                unknown_industry.market_value,
                unknown_industry.weight_percent
            ),
            (0.0, 0.0)
        );
    }

    /// 沒有持股：全部為 0、權重與報酬率為 null。
    #[test]
    fn summarize_empty_portfolio() {
        let summary = summarize(Vec::new());
        assert_eq!((summary.symbols, summary.market_value), (0, 0.0));
        assert_eq!(summary.unrealized_gain_percent, None);
        assert!(summary.industries.is_empty() && summary.holdings.is_empty());
    }
}
