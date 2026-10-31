//! 歸因的計算：每期依產業（與現金）分組，算 Brinson-Fachler 的配置、選股與交互效果，再彙總成
//! 整段、各年度與各產業。純計算，不做 I/O；公式見上層模組說明。

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate};

use super::super::{PeriodRow, portfolio};
use crate::interfaces::web::data_api::dto::{
    BacktestAttributionTotals, BacktestAttributionYear, BacktestIndustryEffect,
};
use crate::interfaces::web::data_api::handlers::round2;

/// 每年列出影響最大的幾個產業。
pub(super) const YEAR_INDUSTRIES: usize = 5;

/// [`BENCHMARK_SQL`](super::BENCHMARK_SQL) 的一列：一個換股日的基準成分股。
#[derive(Debug, Clone, Default, PartialEq, sqlx::FromRow)]
pub(super) struct BenchmarkRow {
    pub(super) date: NaiveDate,
    pub(super) industries: Vec<i32>,
    pub(super) returns: Vec<f64>,
    pub(super) trade_values: Vec<f64>,
}

/// 歸因的類別：產業或策略部位的現金。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Bucket {
    Industry(i32),
    Cash,
}

/// 一期一個類別的權重（0–1）與效果（百分點）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Effect {
    pub(super) weight: f64,
    pub(super) benchmark_weight: f64,
    pub(super) allocation: f64,
    pub(super) selection: f64,
    pub(super) interaction: f64,
}

impl Effect {
    pub(super) fn total(&self) -> f64 {
        self.allocation + self.selection + self.interaction
    }

    fn add(&mut self, other: &Self) {
        self.weight += other.weight;
        self.benchmark_weight += other.benchmark_weight;
        self.allocation += other.allocation;
        self.selection += other.selection;
        self.interaction += other.interaction;
    }
}

/// 一期的歸因：策略與基準的報酬（%）、0050 報酬與各類別的效果。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PeriodAttribution {
    pub(super) date: NaiveDate,
    pub(super) strategy_return: f64,
    pub(super) benchmark_return: f64,
    pub(super) etf_return: Option<f64>,
    pub(super) effects: BTreeMap<Bucket, Effect>,
}

/// 各類別的權重合計與加權報酬合計。
fn group(items: impl Iterator<Item = (Bucket, f64, f64)>) -> BTreeMap<Bucket, (f64, f64)> {
    let mut groups: BTreeMap<Bucket, (f64, f64)> = BTreeMap::new();
    for (bucket, weight, ret) in items {
        let entry = groups.entry(bucket).or_default();
        entry.0 += weight;
        entry.1 += weight * ret;
    }
    groups
}

/// 一期的 Brinson-Fachler 歸因。策略權重為持股比例（見 [`portfolio::shares`]），只算有報酬的持股
/// 並把它們的權重放大到持股比例合計（與平均報酬的算法相同），其餘為現金；基準沒有成分股回 `None`。
pub(super) fn attribute(row: &PeriodRow, benchmark: &BenchmarkRow) -> Option<PeriodAttribution> {
    let total_value: f64 = benchmark.trade_values.iter().sum();
    if total_value <= 0.0 {
        return None;
    }
    let bench = group(
        benchmark
            .industries
            .iter()
            .zip(&benchmark.returns)
            .zip(&benchmark.trade_values)
            .map(|((industry, ret), value)| {
                (Bucket::Industry(*industry), value / total_value, *ret)
            }),
    );
    let benchmark_return: f64 = bench.values().map(|(_, weighted)| weighted).sum();

    let shares = portfolio::shares(row);
    let priced: Vec<(Bucket, f64, f64)> = row
        .returns
        .iter()
        .enumerate()
        .filter_map(|(index, ret)| {
            let industry = row.industries.get(index).copied().unwrap_or(0);
            ret.map(|ret| (Bucket::Industry(industry), shares[index], ret))
        })
        .collect();
    let priced_share: f64 = priced.iter().map(|(_, share, _)| share).sum();
    let invested = if priced_share > 0.0 {
        1.0 - row.cash
    } else {
        0.0
    };
    let mut strategy = group(
        priced
            .into_iter()
            .map(|(bucket, share, ret)| (bucket, share / priced_share * invested, ret)),
    );
    if invested < 1.0 {
        strategy.insert(Bucket::Cash, (1.0 - invested, 0.0));
    }
    let strategy_return: f64 = strategy.values().map(|(_, weighted)| weighted).sum();

    let mut effects = BTreeMap::new();
    for bucket in strategy.keys().chain(bench.keys()) {
        if effects.contains_key(bucket) {
            continue;
        }
        let (wp, weighted_p) = strategy.get(bucket).copied().unwrap_or_default();
        let (wb, weighted_b) = bench.get(bucket).copied().unwrap_or_default();
        let rb = match bucket {
            Bucket::Cash => 0.0,
            Bucket::Industry(_) if wb > 0.0 => weighted_b / wb,
            Bucket::Industry(_) => benchmark_return,
        };
        let rp = if wp > 0.0 { weighted_p / wp } else { rb };
        effects.insert(
            *bucket,
            Effect {
                weight: wp,
                benchmark_weight: wb,
                allocation: (wp - wb) * (rb - benchmark_return),
                selection: wb * (rp - rb),
                interaction: (wp - wb) * (rp - rb),
            },
        );
    }
    Some(PeriodAttribution {
        date: row.date,
        strategy_return,
        benchmark_return,
        etf_return: row.etf_return,
        effects,
    })
}

/// 各期合計：期數、報酬與三項效果相加，以及各類別的效果與權重合計。
pub(super) fn totals(
    periods: &[&PeriodAttribution],
) -> (BacktestAttributionTotals, BTreeMap<Bucket, Effect>) {
    let mut buckets: BTreeMap<Bucket, Effect> = BTreeMap::new();
    for period in periods {
        for (bucket, effect) in &period.effects {
            buckets.entry(*bucket).or_default().add(effect);
        }
    }
    let sum = |pick: fn(&Effect) -> f64| round2(buckets.values().map(pick).sum());
    let strategy_return: f64 = periods.iter().map(|period| period.strategy_return).sum();
    let benchmark_return: f64 = periods.iter().map(|period| period.benchmark_return).sum();
    let has_etf = periods.iter().any(|period| period.etf_return.is_some());
    let totals = BacktestAttributionTotals {
        periods: periods.len() as i64,
        strategy_return: round2(strategy_return),
        benchmark_return: round2(benchmark_return),
        etf_return: has_etf
            .then(|| round2(periods.iter().filter_map(|period| period.etf_return).sum())),
        excess_return: round2(strategy_return - benchmark_return),
        allocation: sum(|effect| effect.allocation),
        selection: sum(|effect| effect.selection),
        interaction: sum(|effect| effect.interaction),
    };
    (totals, buckets)
}

/// 各類別的效果輸出：權重換成各期平均（%），依三項合計由大到小；`top` 有值時改依合計的絕對值
/// 由大到小、只取前 `top` 個。
pub(super) fn industry_effects(
    buckets: &BTreeMap<Bucket, Effect>,
    periods: usize,
    names: &HashMap<i32, String>,
    top: Option<usize>,
) -> Vec<BacktestIndustryEffect> {
    let count = periods.max(1) as f64;
    let mut items: Vec<(f64, BacktestIndustryEffect)> = buckets
        .iter()
        .map(|(bucket, effect)| {
            let industry = match bucket {
                Bucket::Cash => "現金".to_owned(),
                Bucket::Industry(id) => names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "未分類".to_owned()),
            };
            (
                effect.total(),
                BacktestIndustryEffect {
                    industry,
                    avg_weight: round2(effect.weight / count * 100.0),
                    benchmark_avg_weight: round2(effect.benchmark_weight / count * 100.0),
                    allocation: round2(effect.allocation),
                    selection: round2(effect.selection),
                    interaction: round2(effect.interaction),
                    total: round2(effect.total()),
                },
            )
        })
        .collect();
    match top {
        Some(top) => {
            items.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
            items.truncate(top);
        }
        None => items.sort_by(|a, b| b.0.total_cmp(&a.0)),
    }
    items.into_iter().map(|(_, item)| item).collect()
}

/// 依換股日年度的合計與影響最大的 [`YEAR_INDUSTRIES`] 個產業，由舊到新。
pub(super) fn attribution_years(
    periods: &[PeriodAttribution],
    names: &HashMap<i32, String>,
) -> Vec<BacktestAttributionYear> {
    let mut by_year: BTreeMap<i32, Vec<&PeriodAttribution>> = BTreeMap::new();
    for period in periods {
        by_year.entry(period.date.year()).or_default().push(period);
    }
    by_year
        .into_iter()
        .map(|(year, periods)| {
            let (totals, buckets) = totals(&periods);
            BacktestAttributionYear {
                year,
                industries: industry_effects(&buckets, periods.len(), names, Some(YEAR_INDUSTRIES)),
                totals,
            }
        })
        .collect()
}
