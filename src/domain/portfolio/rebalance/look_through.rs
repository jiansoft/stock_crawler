//! ETF 穿透維度：直接持股加上 ETF 依成分股權重穿透的間接持股，與目標比例比較偏離。

use std::collections::{HashMap, HashSet};

use super::{CurrentPosition, EtfLookThroughComponent, EtfLookThroughRebalanceItem, round2};

/// ETF 穿透維度的偏離（依目標比例由高到低、同比例依代號排序）；只列偏離，不給股數。
pub(super) fn rebalance_look_through(
    targets: &[(String, f64)],
    positions: &[CurrentPosition],
    etf_components: &[EtfLookThroughComponent],
    total_portfolio: f64,
) -> Vec<EtfLookThroughRebalanceItem> {
    // 直接持股市值
    let mut direct_values: HashMap<String, (String, f64)> = HashMap::new();
    let mut etf_positions: HashMap<&str, f64> = HashMap::new();

    for pos in positions {
        let val = pos.shares as f64 * pos.current_price;
        if pos.is_etf {
            etf_positions.insert(pos.symbol.as_str(), val);
        } else {
            direct_values.insert(pos.symbol.clone(), (pos.name.clone(), val));
        }
    }

    // 間接持股市值（依成分股權重穿透）
    let mut indirect_values: HashMap<String, (String, f64)> = HashMap::new();
    for comp in etf_components {
        if let Some(&etf_val) = etf_positions.get(comp.etf_symbol.as_str()) {
            let comp_val = etf_val * (comp.weight_percent / 100.0);
            let entry = indirect_values
                .entry(comp.stock_symbol.clone())
                .or_insert_with(|| (comp.name.clone(), 0.0));
            entry.1 += comp_val;
        }
    }

    let mut all_stocks: HashSet<String> = targets.iter().map(|(k, _)| k.clone()).collect();
    all_stocks.extend(direct_values.keys().cloned());
    all_stocks.extend(indirect_values.keys().cloned());

    let target_map: HashMap<&str, f64> = targets.iter().map(|(k, v)| (k.as_str(), *v)).collect();

    let mut etf_items = Vec::new();
    for sym in all_stocks {
        let target_pct = target_map.get(sym.as_str()).copied().unwrap_or(0.0);
        let (direct_name, direct_val) = direct_values
            .get(&sym)
            .cloned()
            .unwrap_or((sym.clone(), 0.0));
        let (indirect_name, indirect_val) = indirect_values
            .get(&sym)
            .cloned()
            .unwrap_or((sym.clone(), 0.0));
        let name = if !direct_name.is_empty() && direct_name != sym {
            direct_name
        } else {
            indirect_name
        };

        let total_val = direct_val + indirect_val;
        let current_pct = (total_val / total_portfolio) * 100.0;
        let deviation = target_pct - current_pct;

        etf_items.push(EtfLookThroughRebalanceItem {
            stock_symbol: sym,
            name,
            direct_value: round2(direct_val),
            indirect_value: round2(indirect_val),
            total_value: round2(total_val),
            current_weight: round2(current_pct),
            target_weight: round2(target_pct),
            deviation_percent: round2(deviation),
        });
    }

    etf_items.sort_by(|a, b| {
        b.target_weight
            .partial_cmp(&a.target_weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    etf_items
}
