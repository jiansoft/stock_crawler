//! 產業維度：直接持股依產業加總，ETF 市值依成分股產業拆分後，與目標比例比較偏離。

use std::collections::HashMap;

use super::{CurrentPosition, EtfLookThroughComponent, IndustryRebalanceItem, round2};

/// 產業維度的偏離（依目標比例由高到低、同比例依產業名稱排序）；只列偏離，不給股數。
pub(super) fn rebalance_industries(
    targets: &[(String, f64)],
    positions: &[CurrentPosition],
    etf_components: &[EtfLookThroughComponent],
    total_portfolio: f64,
) -> Vec<IndustryRebalanceItem> {
    let mut ind_values: HashMap<String, f64> = HashMap::new();
    let mut ind_stocks: HashMap<String, Vec<String>> = HashMap::new();

    for pos in positions.iter().filter(|pos| !pos.is_etf) {
        let ind = if pos.industry.is_empty() {
            "其他".to_string()
        } else {
            pos.industry.clone()
        };
        let val = pos.shares as f64 * pos.current_price;
        *ind_values.entry(ind.clone()).or_default() += val;
        ind_stocks.entry(ind).or_default().push(pos.symbol.clone());
    }

    // ETF 市值依成分股產業拆分；未揭露部分保留在涵蓋率資訊，不假裝已知產業。
    for comp in etf_components {
        let etf_value: f64 = positions
            .iter()
            .filter(|p| p.is_etf && p.symbol == comp.etf_symbol)
            .map(|p| p.shares as f64 * p.current_price)
            .sum();
        let industry = if comp.industry.is_empty() {
            "其他"
        } else {
            &comp.industry
        };
        *ind_values.entry(industry.to_owned()).or_default() +=
            etf_value * comp.weight_percent / 100.0;
    }
    let mut all_inds: Vec<String> = targets.iter().map(|(k, _)| k.clone()).collect();
    for ind in ind_values.keys() {
        if !all_inds.contains(ind) {
            all_inds.push(ind.clone());
        }
    }

    let target_map: HashMap<&str, f64> = targets.iter().map(|(k, v)| (k.as_str(), *v)).collect();

    let mut industry_items = Vec::new();
    for ind in all_inds {
        let target_pct = target_map.get(ind.as_str()).copied().unwrap_or(0.0);
        let current_val = ind_values.get(&ind).copied().unwrap_or(0.0);
        let current_pct = (current_val / total_portfolio) * 100.0;
        let deviation = target_pct - current_pct;
        let mut candidate_stocks = ind_stocks.get(&ind).cloned().unwrap_or_default();
        candidate_stocks.sort();

        industry_items.push(IndustryRebalanceItem {
            industry: ind,
            current_value: round2(current_val),
            current_weight: round2(current_pct),
            target_weight: round2(target_pct),
            deviation_percent: round2(deviation),
            candidate_stocks,
        });
    }

    industry_items.sort_by(|a, b| {
        b.target_weight
            .partial_cmp(&a.target_weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.industry.cmp(&b.industry))
    });
    industry_items
}
