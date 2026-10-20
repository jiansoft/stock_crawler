//! 市場估值水位：上市櫃官方本益比與股價淨值比的每日中位數。
//!
//! 每日中位數預先算好存在 `daily_market_valuation`（只計大於 0 的值：虧損股沒有本益比、ETF 兩者都沒有）。
//! 這裡放讀寫介面與分布計算：樣本數不到期間每日樣本數中位數一半的日子（資料寫入到一半或缺漏）不列入，
//! 再看最新一天的中位數落在期間的哪個位置。`/market/valuation`、`/market/regime`、每週市場摘要、大盤趨勢
//! 通知與回測的大盤估值濾網共用。

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Months, NaiveDate};

/// 上市加上櫃合計的市場編號（上市 2、上櫃 4）。
pub const ALL_MARKETS: i32 = 0;

/// 單一交易日、單一市場的本益比與股價淨值比中位數。
#[derive(Debug, Clone, PartialEq)]
pub struct MarketValuationDay {
    /// 交易日。
    pub date: NaiveDate,
    /// 本益比中位數；當天沒有時為 `None`。
    pub median_pe: Option<f64>,
    /// 股價淨值比中位數；當天沒有時為 `None`。
    pub median_pb: Option<f64>,
    /// 計入本益比中位數的股票數。
    pub pe_samples: i64,
    /// 計入股價淨值比中位數的股票數。
    pub pb_samples: i64,
}

/// 期間內每日中位數的分布與最新一天的位置；數值不四捨五入。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValuationDistribution {
    /// 最新一天的中位數。
    pub current: f64,
    /// 期間內不高於最新一天的交易日比例（%，0–100）。
    pub percentile: f64,
    /// 期間最低。
    pub min: f64,
    /// 第 25 百分位。
    pub p25: f64,
    /// 中位數。
    pub median: f64,
    /// 第 75 百分位。
    pub p75: f64,
    /// 期間最高。
    pub max: f64,
    /// 參與計算的交易日數。
    pub days: usize,
}

/// 市場估值水位的讀寫介面。
#[async_trait]
pub trait MarketValuationRepository: Send + Sync {
    /// 由日線重算 `from`～`to` 每個交易日上市、上櫃與合計的中位數並寫入（同一天同市場覆蓋），回傳寫入列數。
    async fn refresh_market_valuation(&self, from: NaiveDate, to: NaiveDate) -> Result<u64>;

    /// 讀取 `market_id`（[`ALL_MARKETS`]、2 或 4）在 `from`～`to` 的每日中位數，由舊到新。
    async fn fetch_market_valuation(
        &self,
        market_id: i32,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<MarketValuationDay>>;
}

/// 去掉樣本數不到每日樣本數中位數一半的日子（資料寫入到一半或缺漏）。
pub fn complete_days(days: Vec<MarketValuationDay>) -> Vec<MarketValuationDay> {
    let mut samples: Vec<i64> = days
        .iter()
        .map(|day| day.pe_samples.max(day.pb_samples))
        .collect();
    if samples.is_empty() {
        return days;
    }
    samples.sort_unstable();
    let typical = samples[samples.len() / 2];
    days.into_iter()
        .filter(|day| day.pe_samples.max(day.pb_samples) * 2 >= typical)
        .collect()
}

/// 每日中位數的分布與 `current` 的百分位（不高於它的日子占比）；`current` 或資料為空時回 `None`。
pub fn distribution(mut values: Vec<f64>, current: Option<f64>) -> Option<ValuationDistribution> {
    let current = current?;
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let not_above = values.iter().filter(|value| **value <= current).count();
    Some(ValuationDistribution {
        current,
        percentile: not_above as f64 / values.len() as f64 * 100.0,
        min: values[0],
        p25: quantile(&values, 0.25),
        median: quantile(&values, 0.5),
        p75: quantile(&values, 0.75),
        max: values[values.len() - 1],
        days: values.len(),
    })
}

/// 估值水位回看的年數。
pub const LEVEL_YEARS: u32 = 5;

/// 上市櫃本益比、股價淨值比中位數在近 [`LEVEL_YEARS`] 年的位置。
#[derive(Debug, Clone, PartialEq)]
pub struct ValuationLevel {
    /// 最新一個有估值資料的交易日。
    pub date: NaiveDate,
    /// 本益比中位數的分布；最新一天沒有時為 `None`。
    pub pe: Option<ValuationDistribution>,
    /// 股價淨值比中位數的分布；最新一天沒有時為 `None`。
    pub pb: Option<ValuationDistribution>,
    /// [`Self::before_days`] 個交易日前的本益比中位數；資料不夠時為 `None`。
    pub pe_before: Option<f64>,
    /// `pe_before` 是幾個交易日前。
    pub before_days: usize,
}

impl ValuationLevel {
    /// 一行摘要，例如「本益比中位數 17.87 倍（近 5 年第 34 百分位，5 日前 18.10 倍）、股價淨值比中位數
    /// 1.78 倍（第 48 百分位）」；兩者都沒有時回 `None`。
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(pe) = &self.pe {
            let before = self.pe_before.map_or_else(String::new, |value| {
                format!("，{} 日前 {value:.2} 倍", self.before_days)
            });
            parts.push(format!(
                "本益比中位數 {:.2} 倍（近 {LEVEL_YEARS} 年第 {:.0} 百分位{before}）",
                pe.current, pe.percentile
            ));
        }
        if let Some(pb) = &self.pb {
            parts.push(format!(
                "股價淨值比中位數 {:.2} 倍（第 {:.0} 百分位）",
                pb.current, pb.percentile
            ));
        }
        (!parts.is_empty()).then(|| parts.join("、"))
    }
}

/// `date`（含）之前近 [`LEVEL_YEARS`] 年的上市櫃估值水位，另附 `before_days` 個交易日前的本益比中位數；
/// 沒有資料回 `None`。
///
/// # Errors
///
/// 讀取每日中位數失敗時回傳錯誤。
pub async fn level(
    repo: &impl MarketValuationRepository,
    date: NaiveDate,
    before_days: usize,
) -> Result<Option<ValuationLevel>> {
    let since = date
        .checked_sub_months(Months::new(LEVEL_YEARS * 12))
        .unwrap_or(NaiveDate::MIN);
    let days = complete_days(
        repo.fetch_market_valuation(ALL_MARKETS, since, date)
            .await?,
    );
    let Some(latest) = days.last() else {
        return Ok(None);
    };
    let pe_before = days
        .len()
        .checked_sub(before_days + 1)
        .and_then(|index| days[index].median_pe);
    Ok(Some(ValuationLevel {
        date: latest.date,
        pe: distribution(
            days.iter().filter_map(|day| day.median_pe).collect(),
            latest.median_pe,
        ),
        pb: distribution(
            days.iter().filter_map(|day| day.median_pb).collect(),
            latest.median_pb,
        ),
        pe_before,
        before_days,
    }))
}

/// 已排序資料的分位數，線性內插（同 PostgreSQL `percentile_cont`）。
pub fn quantile(sorted: &[f64], fraction: f64) -> f64 {
    let position = fraction * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32, pe: Option<f64>, samples: i64) -> MarketValuationDay {
        MarketValuationDay {
            date: NaiveDate::from_ymd_opt(2026, 1, d).expect("合法日期"),
            median_pe: pe,
            median_pb: None,
            pe_samples: samples,
            pb_samples: samples,
        }
    }

    /// 分位數線性內插：與 percentile_cont 相同。
    #[test]
    fn quantile_interpolates_linearly() {
        let values = [10.0, 12.0, 14.0, 20.0];
        assert_eq!(quantile(&values, 0.0), 10.0);
        assert_eq!(quantile(&values, 0.5), 13.0);
        assert_eq!(quantile(&values, 0.25), 11.5);
        assert_eq!(quantile(&values, 1.0), 20.0);
        assert_eq!(quantile(&[7.0], 0.75), 7.0);
    }

    /// 百分位是不高於最新一天的日子占比；沒有最新值或沒有資料時不算。
    #[test]
    fn distribution_reports_quartiles_and_percentile() {
        let stats = distribution(vec![14.0, 10.0, 20.0, 12.0], Some(14.0)).expect("有資料");
        assert_eq!(
            stats,
            ValuationDistribution {
                current: 14.0,
                percentile: 75.0,
                min: 10.0,
                p25: 11.5,
                median: 13.0,
                p75: 15.5,
                max: 20.0,
                days: 4,
            }
        );
        assert!(distribution(vec![10.0], None).is_none());
        assert!(distribution(Vec::new(), Some(10.0)).is_none());
    }

    /// 樣本數不到典型值一半的日子被去掉；空資料原樣回傳。
    #[test]
    fn complete_days_drops_partial_days() {
        let kept = complete_days(vec![
            day(2, Some(15.0), 1_000),
            day(5, Some(15.5), 990),
            day(6, Some(40.0), 30),
        ]);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|day| day.pe_samples >= 990));
        assert!(complete_days(Vec::new()).is_empty());
    }
}
