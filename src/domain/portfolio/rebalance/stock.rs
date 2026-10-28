//! 個股維度：依目標比例算出每檔的買賣股數、交易成本，以及賣出淨額、買進總額與剩餘現金的結算。

use std::collections::HashMap;

use super::{CurrentPosition, RebalanceCashSummary, RebalanceParams, StockRebalanceItem, round2};

/// 個股維度的調整建議（依目標比例由高到低、同比例依代號排序）與現金結算。
pub(super) fn rebalance_stocks(
    targets: &[(String, f64)],
    positions: &[CurrentPosition],
    total_portfolio: f64,
    params: &RebalanceParams,
) -> (Vec<StockRebalanceItem>, RebalanceCashSummary) {
    let lot_size = params.lot_size;
    let mut pos_map: HashMap<&str, &CurrentPosition> = HashMap::new();
    for pos in positions {
        pos_map.insert(pos.symbol.as_str(), pos);
    }

    let mut all_keys: Vec<String> = targets.iter().map(|(k, _)| k.clone()).collect();
    for pos in positions {
        if !all_keys.contains(&pos.symbol) {
            all_keys.push(pos.symbol.clone());
        }
    }

    let target_map: HashMap<&str, f64> = targets.iter().map(|(k, v)| (k.as_str(), *v)).collect();

    let mut stock_items = Vec::new();
    let mut total_sell_net = 0.0_f64;
    let mut total_buy_cost = 0.0_f64;

    for key in all_keys {
        let target_pct = target_map.get(key.as_str()).copied().unwrap_or(0.0);
        let pos_opt = pos_map.get(key.as_str()).copied();

        let (name, shares, price) = match pos_opt {
            Some(p) => (p.name.clone(), p.shares, p.current_price),
            None => (key.clone(), 0, 0.0),
        };

        let current_val = shares as f64 * price;
        let current_pct = (current_val / total_portfolio) * 100.0;
        let deviation = target_pct - current_pct;

        let mut action = "hold";
        let mut suggested_shares = 0_i64;
        let mut suggested_amount = 0.0_f64;
        let mut estimated_cost = 0.0_f64;
        let mut net_amount = 0.0_f64;

        if price > 0.0 && deviation.abs() >= params.tolerance_percent {
            let lot = f64::from(lot_size);
            if deviation > 0.0 {
                // 買入
                action = "buy";
                let target_val_diff = (deviation / 100.0) * total_portfolio;
                let raw_shares = (target_val_diff / price / lot).round() * lot;
                suggested_shares = raw_shares.max(0.0) as i64;
                suggested_amount = suggested_shares as f64 * price;

                if suggested_shares > 0 {
                    let fee = (suggested_amount * params.fee_rate * params.fee_discount).max(20.0);
                    estimated_cost = round2(fee);
                    net_amount = round2(suggested_amount + estimated_cost);
                    total_buy_cost += net_amount;
                }
            } else {
                // 賣出（不得超過持有股數）
                action = "sell";
                let target_val_diff = (-deviation / 100.0) * total_portfolio;
                let raw_shares = (target_val_diff / price / lot).round() * lot;
                let capped_shares = (raw_shares.max(0.0) as i64).min(shares);

                suggested_shares = if lot_size > 1 {
                    (capped_shares / lot_size as i64) * lot_size as i64
                } else {
                    capped_shares
                };

                if suggested_shares > 0 {
                    suggested_amount = suggested_shares as f64 * price;
                    let fee = (suggested_amount * params.fee_rate * params.fee_discount).max(20.0);
                    let tax_rate = if pos_opt.is_some_and(|p| p.is_etf) {
                        super::super::costs::ETF_TAX_RATE
                    } else {
                        params.tax_rate
                    };
                    let tax = suggested_amount * tax_rate;
                    estimated_cost = round2(fee + tax);
                    net_amount = round2((suggested_amount - estimated_cost).max(0.0));
                    total_sell_net += net_amount;
                } else {
                    action = "hold";
                }
            }
        }

        stock_items.push(StockRebalanceItem {
            symbol: key,
            name,
            current_shares: shares,
            current_price: round2(price),
            current_value: round2(current_val),
            current_weight: round2(current_pct),
            target_weight: round2(target_pct),
            deviation_percent: round2(deviation),
            action,
            suggested_shares,
            suggested_amount: round2(suggested_amount),
            estimated_cost,
            net_amount,
        });
    }

    stock_items.sort_by(|a, b| {
        b.target_weight
            .partial_cmp(&a.target_weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });

    let available_capital = params.available_cash + total_sell_net;
    let required_additional_cash = (total_buy_cost - available_capital).max(0.0);
    let remaining_cash = (available_capital - total_buy_cost).max(0.0);

    let cash_summary = RebalanceCashSummary {
        initial_cash: round2(params.available_cash),
        estimated_sell_net: round2(total_sell_net),
        estimated_buy_total: round2(total_buy_cost),
        available_capital: round2(available_capital),
        required_additional_cash: round2(required_additional_cash),
        remaining_cash: round2(remaining_cash),
    };
    (stock_items, cash_summary)
}
