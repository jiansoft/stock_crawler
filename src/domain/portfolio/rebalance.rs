//! # 投資組合再平衡純計算函式（Portfolio Rebalance）
//!
//! 提供個股（Stock）、產業（Industry）與 ETF 穿透（EtfLookThrough）三種維度的資產配置偏離試算
//! 與具體股數／金額調整建議。本模組為純函式與無狀態計算，易於單元測試。

use std::collections::{HashMap, HashSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub use super::target::TargetDimension;

/// 免責聲明文字。
pub const REBALANCE_DISCLAIMER: &str = "試算結果僅供參考，不構成任何投資建議。實際成交價格、交易成本與稅費依券商及交易所實際撮合結果為準。";

/// 進行再平衡試算的一檔目前持股。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrentPosition {
    /// 股票代號。
    pub symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 現有持有股數。
    pub shares: i64,
    /// 目前每股市價（收盤價）。
    pub current_price: f64,
    /// 產業名稱。
    pub industry: String,
    /// 是否為 ETF；由股票主檔／ETF 資料判定，不猜代號前綴。
    pub is_etf: bool,
}

/// ETF 穿透成分股。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EtfLookThroughComponent {
    /// 成分股股票代號。
    pub stock_symbol: String,
    /// 成分股名稱。
    pub name: String,
    /// 成分股占該 ETF 的權重百分比（0.0 ~ 100.0）。
    pub weight_percent: f64,
    /// 所屬 ETF 代號。
    pub etf_symbol: String,
    /// 成分股的產業，用於產業維度穿透。
    pub industry: String,
}

/// 再平衡試算參數。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebalanceParams {
    /// 交易單位：1（零股）或 1000（整張），預設 1000。
    pub lot_size: u32,
    /// 容忍帶百分比（例如 2.0 代表偏離在 ±2.0% 內不建議交易），預設 2.0。
    pub tolerance_percent: f64,
    /// 目前帳戶可用現金（元），預設 0.0。
    pub available_cash: f64,
    /// 手續費率，預設 0.001425。
    pub fee_rate: f64,
    /// 證券交易稅率，預設 0.003。
    pub tax_rate: f64,
    /// 手續費折讓（例如 0.28 代表 28 折，1.0 為無折讓），預設 1.0。
    pub fee_discount: f64,
}

impl Default for RebalanceParams {
    fn default() -> Self {
        Self {
            lot_size: 1000,
            tolerance_percent: 2.0,
            available_cash: 0.0,
            fee_rate: super::costs::FEE_RATE,
            tax_rate: super::costs::STOCK_TAX_RATE,
            fee_discount: 1.0,
        }
    }
}

/// 個股維度再平衡建議項目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockRebalanceItem {
    /// 股票代號。
    pub symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 現有持有股數。
    pub current_shares: i64,
    /// 目前市價。
    pub current_price: f64,
    /// 現有市值（元）。
    pub current_value: f64,
    /// 現有權重百分比（占總投資組合市值）。
    pub current_weight: f64,
    /// 目標權重百分比。
    pub target_weight: f64,
    /// 偏離百分點（目標 − 目前；正值代表偏低需加碼，負值代表偏高需減碼）。
    pub deviation_percent: f64,
    /// 建議動作：`buy`、`sell` 或 `hold`。
    pub action: &'static str,
    /// 建議調整股數（正數，若為 hold 則為 0）。
    pub suggested_shares: i64,
    /// 預估交易金額（股數 × 目前市價）。
    pub suggested_amount: f64,
    /// 預估交易成本（手續費 ＋ 證交稅，手續費含低消 20 元）。
    pub estimated_cost: f64,
    /// 預估交易淨額（買入為 金額 ＋ 費用；賣出為 金額 − 費用）。
    pub net_amount: f64,
}

/// 產業維度再平衡建議項目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndustryRebalanceItem {
    /// 產業名稱。
    pub industry: String,
    /// 現有市值（元）。
    pub current_value: f64,
    /// 現有權重百分比。
    pub current_weight: f64,
    /// 目標權重百分比。
    pub target_weight: f64,
    /// 偏離百分點。
    pub deviation_percent: f64,
    /// 現有持股中屬於該產業的標的清單（作為建議加減碼之候選）。
    pub candidate_stocks: Vec<String>,
}

/// ETF 穿透維度再平衡建議項目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EtfLookThroughRebalanceItem {
    /// 成分股股票代號。
    pub stock_symbol: String,
    /// 成分股股票名稱。
    pub name: String,
    /// 直接持有市值（元）。
    pub direct_value: f64,
    /// 透過 ETF 間接持有市值（元）。
    pub indirect_value: f64,
    /// 穿透後合計總市值（元）。
    pub total_value: f64,
    /// 穿透後現有權重百分比。
    pub current_weight: f64,
    /// 目標權重百分比。
    pub target_weight: f64,
    /// 偏離百分點。
    pub deviation_percent: f64,
}

/// 現金流平衡試算摘要。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebalanceCashSummary {
    /// 期初可用現金（元）。
    pub initial_cash: f64,
    /// 預估賣出實收淨額合計（元）。
    pub estimated_sell_net: f64,
    /// 預估買入需支付總額合計（元）。
    pub estimated_buy_total: f64,
    /// 可用資金總額（期初現金 ＋ 賣出實收淨額）。
    pub available_capital: f64,
    /// 需新增資金（若買入總額大於可用資金，則為不足之金額；否則為 0）。
    pub required_additional_cash: f64,
    /// 交易後預估剩餘現金（元）。
    pub remaining_cash: f64,
}

/// 投資組合再平衡試算完整結果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioRebalanceResult {
    /// 試算維度。
    pub dimension: TargetDimension,
    /// 投資組合股票總市值（元）。
    pub total_equity_value: f64,
    /// 投資組合總資產市值（股票市值 ＋ 可用現金）。
    pub total_portfolio_value: f64,
    /// 目標現金或未指定比例（100% − 各標的目標權重總和）。
    pub target_cash_percent: f64,
    /// 採用之容忍帶（%）。
    pub tolerance_percent: f64,
    /// 採用之交易單位股數（1 或 1000）。
    pub lot_size: u32,
    /// 個股維度建議項目（僅在 dimension 為 Stock 時有值）。
    pub stock_items: Vec<StockRebalanceItem>,
    /// 產業維度建議項目（僅在 dimension 為 Industry 時有值）。
    pub industry_items: Vec<IndustryRebalanceItem>,
    /// ETF 穿透維度建議項目（僅在 dimension 為 EtfLookThrough 時有值）。
    pub etf_items: Vec<EtfLookThroughRebalanceItem>,
    /// 現金流試算摘要。
    pub cash_summary: RebalanceCashSummary,
    /// 免責聲明文字。
    pub disclaimer: &'static str,
}

/// 金額與百分比統一四捨五入至兩位小數。
fn round2(val: f64) -> f64 {
    (val * 100.0).round() / 100.0
}

/// 載入目前持股（含收盤價與產業）的查詢介面，供週報等非 HTTP 呼叫端重用。
#[async_trait::async_trait]
pub trait PortfolioPositionQuery: Send + Sync {
    /// 回傳 `member_id` 的未售出持股（`0` 為全部會員）；`extra_symbols` 內未持有的代號
    /// 以零股數補上，供目標中尚未持有的標的試算加碼。收盤價缺漏時 `current_price` 為 0。
    async fn fetch_positions(
        &self,
        member_id: i64,
        extra_symbols: &[String],
    ) -> Result<Vec<CurrentPosition>>;
}

/// 計算持股再平衡建議之純函式。
///
/// # 參數
/// - `dimension`: 試算維度（個股、產業、ETF穿透）
/// - `targets`: 使用者設定之目標比例清單 `(key, target_percent)`
/// - `positions`: 目前持股明細
/// - `etf_components`: ETF 成分股穿透資料（僅在 ETF 穿透維度使用）
/// - `params`: 再平衡參數（容忍帶、股數單位、現有現金、手續費等）
///
/// # 錯誤處理
/// - 若目標權重總和超過 100.0%，回傳錯誤拒絕。
/// - 若任一權重小於 0% 或大於 100%，回傳錯誤拒絕。
pub fn calculate_rebalance(
    dimension: TargetDimension,
    targets: &[(String, f64)],
    positions: &[CurrentPosition],
    etf_components: &[EtfLookThroughComponent],
    params: &RebalanceParams,
) -> Result<PortfolioRebalanceResult> {
    super::target::validate_targets(targets)?;
    let target_sum: f64 = targets.iter().map(|(_, pct)| pct).sum();
    ensure!(
        matches!(params.lot_size, 1 | 1000),
        "交易單位須為 1 或 1000"
    );
    ensure!(
        params.tolerance_percent.is_finite() && (0.0..=100.0).contains(&params.tolerance_percent),
        "容忍帶須為 0 至 100 的有限數值"
    );
    ensure!(
        params.available_cash.is_finite() && params.available_cash >= 0.0,
        "現金須為非負有限數值"
    );
    ensure!(
        [params.fee_rate, params.tax_rate, params.fee_discount]
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "費率須為 0 至 1 的有限數值"
    );
    for position in positions {
        ensure!(
            position.shares >= 0
                && position.current_price.is_finite()
                && position.current_price > 0.0,
            "{} 缺少有效收盤價或股數不合法",
            position.symbol
        );
    }
    let mut coverage = HashMap::<&str, f64>::new();
    for comp in etf_components {
        ensure!(
            comp.weight_percent.is_finite() && (0.0..=100.0).contains(&comp.weight_percent),
            "ETF 成分權重不合法"
        );
        *coverage.entry(&comp.etf_symbol).or_default() += comp.weight_percent;
    }
    ensure!(
        coverage.values().all(|v| *v <= 100.000001),
        "ETF 成分權重合計超過 100%"
    );
    let lot_size = params.lot_size;
    let total_equity: f64 = positions
        .iter()
        .map(|p| p.shares as f64 * p.current_price)
        .sum();
    let total_portfolio = total_equity + params.available_cash;
    let target_cash_percent = (100.0 - target_sum).max(0.0);

    let empty_cash_summary = RebalanceCashSummary {
        initial_cash: round2(params.available_cash),
        estimated_sell_net: 0.0,
        estimated_buy_total: 0.0,
        available_capital: round2(params.available_cash),
        required_additional_cash: 0.0,
        remaining_cash: round2(params.available_cash),
    };

    if total_portfolio <= 0.0 {
        return Ok(PortfolioRebalanceResult {
            dimension,
            total_equity_value: 0.0,
            total_portfolio_value: 0.0,
            target_cash_percent: round2(target_cash_percent),
            tolerance_percent: params.tolerance_percent,
            lot_size,
            stock_items: Vec::new(),
            industry_items: Vec::new(),
            etf_items: Vec::new(),
            cash_summary: empty_cash_summary,
            disclaimer: REBALANCE_DISCLAIMER,
        });
    }

    match dimension {
        TargetDimension::Stock => {
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

            let target_map: HashMap<&str, f64> =
                targets.iter().map(|(k, v)| (k.as_str(), *v)).collect();

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
                            let fee = (suggested_amount * params.fee_rate * params.fee_discount)
                                .max(20.0);
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
                            let fee = (suggested_amount * params.fee_rate * params.fee_discount)
                                .max(20.0);
                            let tax_rate = if pos_opt.is_some_and(|p| p.is_etf) {
                                super::costs::ETF_TAX_RATE
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

            Ok(PortfolioRebalanceResult {
                dimension,
                total_equity_value: round2(total_equity),
                total_portfolio_value: round2(total_portfolio),
                target_cash_percent: round2(target_cash_percent),
                tolerance_percent: params.tolerance_percent,
                lot_size,
                stock_items,
                industry_items: Vec::new(),
                etf_items: Vec::new(),
                cash_summary,
                disclaimer: REBALANCE_DISCLAIMER,
            })
        }
        TargetDimension::Industry => {
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

            let target_map: HashMap<&str, f64> =
                targets.iter().map(|(k, v)| (k.as_str(), *v)).collect();

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

            Ok(PortfolioRebalanceResult {
                dimension,
                total_equity_value: round2(total_equity),
                total_portfolio_value: round2(total_portfolio),
                target_cash_percent: round2(target_cash_percent),
                tolerance_percent: params.tolerance_percent,
                lot_size,
                stock_items: Vec::new(),
                industry_items,
                etf_items: Vec::new(),
                cash_summary: empty_cash_summary,
                disclaimer: REBALANCE_DISCLAIMER,
            })
        }
        TargetDimension::EtfLookThrough => {
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

            let target_map: HashMap<&str, f64> =
                targets.iter().map(|(k, v)| (k.as_str(), *v)).collect();

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

            Ok(PortfolioRebalanceResult {
                dimension,
                total_equity_value: round2(total_equity),
                total_portfolio_value: round2(total_portfolio),
                target_cash_percent: round2(target_cash_percent),
                tolerance_percent: params.tolerance_percent,
                lot_size,
                stock_items: Vec::new(),
                industry_items: Vec::new(),
                etf_items,
                cash_summary: empty_cash_summary,
                disclaimer: REBALANCE_DISCLAIMER,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_positions() -> Vec<CurrentPosition> {
        vec![
            CurrentPosition {
                symbol: "2330".to_string(),
                name: "台積電".to_string(),
                shares: 10_000,
                current_price: 1000.0,
                industry: "半導體業".to_string(),
                is_etf: false,
            },
            CurrentPosition {
                symbol: "2317".to_string(),
                name: "鴻海".to_string(),
                shares: 20_000,
                current_price: 200.0,
                industry: "其他電子業".to_string(),
                is_etf: false,
            },
        ]
    }

    #[test]
    fn rebalance_tolerance_band_prevents_small_trades() {
        // 總市值：10000*1000 + 20000*200 = 10,000,000 + 4,000,000 = 14,000,000
        // 台積電權重：10M / 14M = 71.43%
        // 若目標設為 72.0%（偏離 +0.57% < tolerance 2.0%），應建議 hold 不下單
        let targets = vec![("2330".to_string(), 72.0), ("2317".to_string(), 28.0)];
        let params = RebalanceParams {
            tolerance_percent: 2.0,
            ..Default::default()
        };

        let result = calculate_rebalance(
            TargetDimension::Stock,
            &targets,
            &sample_positions(),
            &[],
            &params,
        )
        .expect("試算成功");

        let tsmc = result
            .stock_items
            .iter()
            .find(|i| i.symbol == "2330")
            .unwrap();
        assert_eq!(tsmc.action, "hold");
        assert_eq!(tsmc.suggested_shares, 0);
    }

    #[test]
    fn rebalance_sell_capped_by_held_shares() {
        // 台積電現有 10,000 股，若目標權重設為 0%，建議賣出股數不得超過現有 10,000 股
        let targets = vec![("2330".to_string(), 0.0), ("2317".to_string(), 100.0)];
        let params = RebalanceParams::default();

        let result = calculate_rebalance(
            TargetDimension::Stock,
            &targets,
            &sample_positions(),
            &[],
            &params,
        )
        .expect("試算成功");

        let tsmc = result
            .stock_items
            .iter()
            .find(|i| i.symbol == "2330")
            .unwrap();
        assert_eq!(tsmc.action, "sell");
        assert!(tsmc.suggested_shares <= tsmc.current_shares);
        assert_eq!(tsmc.suggested_shares, 10_000);
    }

    #[test]
    fn rebalance_lot_size_odd_vs_round_lot() {
        // 當 lot_size = 1 時，股數不強制為 1000 之倍數；當 lot_size = 1000 時必為 1000 倍數
        let targets = vec![("2330".to_string(), 50.0), ("2317".to_string(), 50.0)];

        let params_round = RebalanceParams {
            lot_size: 1000,
            ..Default::default()
        };
        let res_round = calculate_rebalance(
            TargetDimension::Stock,
            &targets,
            &sample_positions(),
            &[],
            &params_round,
        )
        .unwrap();
        for item in &res_round.stock_items {
            assert_eq!(item.suggested_shares % 1000, 0);
        }

        let params_odd = RebalanceParams {
            lot_size: 1,
            ..Default::default()
        };
        let res_odd = calculate_rebalance(
            TargetDimension::Stock,
            &targets,
            &sample_positions(),
            &[],
            &params_odd,
        )
        .unwrap();
        // 零股模式計算正常
        assert!(res_odd.stock_items.iter().any(|i| i.suggested_shares > 0));
    }

    #[test]
    fn rebalance_cash_shortage_reports_additional_cash() {
        // 當買入標的所需總資金超過可用資本（現有現金 ＋ 賣出淨額）時，
        // cash_summary 應正確計算出不足需新增之金額（required_additional_cash > 0）
        let targets = vec![("2330".to_string(), 100.0)];
        let positions = vec![CurrentPosition {
            symbol: "2330".to_string(),
            name: "台積電".to_string(),
            shares: 0,
            current_price: 1000.0,
            industry: "半導體業".to_string(),
            is_etf: false,
        }];
        let params = RebalanceParams {
            available_cash: 500_000.0, // 總資產 50 萬，目標 100% 買 1 張台積電需約 100 萬
            ..Default::default()
        };

        let result =
            calculate_rebalance(TargetDimension::Stock, &targets, &positions, &[], &params)
                .unwrap();

        assert_eq!(result.stock_items[0].suggested_shares, 1000);
        assert!(result.cash_summary.required_additional_cash > 0.0);
        assert_eq!(result.cash_summary.remaining_cash, 0.0);
    }

    #[test]
    fn rebalance_rejects_target_sum_exceeding_100() {
        let targets = vec![("2330".to_string(), 70.0), ("2317".to_string(), 35.0)];
        let params = RebalanceParams::default();
        let err = calculate_rebalance(
            TargetDimension::Stock,
            &targets,
            &sample_positions(),
            &[],
            &params,
        )
        .unwrap_err();
        assert!(err.to_string().contains("不得超過 100%"));
    }

    #[test]
    fn rebalance_empty_portfolio_handled_gracefully() {
        let targets = vec![("2330".to_string(), 50.0)];
        let params = RebalanceParams::default();
        let result =
            calculate_rebalance(TargetDimension::Stock, &targets, &[], &[], &params).unwrap();
        assert_eq!(result.total_portfolio_value, 0.0);
        assert!(result.stock_items.is_empty());
    }

    #[test]
    fn rebalance_industry_dimension_includes_candidates() {
        let targets = vec![
            ("半導體業".to_string(), 50.0),
            ("其他電子業".to_string(), 50.0),
        ];
        let params = RebalanceParams::default();
        let result = calculate_rebalance(
            TargetDimension::Industry,
            &targets,
            &sample_positions(),
            &[],
            &params,
        )
        .unwrap();

        assert_eq!(result.industry_items.len(), 2);
        let semi = result
            .industry_items
            .iter()
            .find(|i| i.industry == "半導體業")
            .unwrap();
        assert_eq!(semi.candidate_stocks, vec!["2330"]);
    }

    #[test]
    fn rebalance_etf_look_through_aggregates_direct_and_indirect() {
        let positions = vec![
            CurrentPosition {
                symbol: "2330".to_string(),
                name: "台積電".to_string(),
                shares: 1_000,
                current_price: 1000.0, // 直接持股 1,000,000
                industry: "半導體業".to_string(),
                is_etf: false,
            },
            CurrentPosition {
                symbol: "0050".to_string(),
                name: "元大台灣50".to_string(),
                shares: 10_000,
                current_price: 200.0, // ETF 持股 2,000,000
                industry: "ETF".to_string(),
                is_etf: true,
            },
        ];

        let etf_components = vec![EtfLookThroughComponent {
            stock_symbol: "2330".to_string(),
            name: "台積電".to_string(),
            weight_percent: 55.0, // 0050 內台積電占 55% -> 間接持有 2,000,000 * 55% = 1,100,000
            etf_symbol: "0050".to_string(),
            industry: "半導體業".to_string(),
        }];

        let targets = vec![("2330".to_string(), 60.0)];
        let params = RebalanceParams::default();
        let result = calculate_rebalance(
            TargetDimension::EtfLookThrough,
            &targets,
            &positions,
            &etf_components,
            &params,
        )
        .unwrap();

        let tsmc = result
            .etf_items
            .iter()
            .find(|i| i.stock_symbol == "2330")
            .unwrap();
        assert_eq!(tsmc.direct_value, 1_000_000.0);
        assert_eq!(tsmc.indirect_value, 1_100_000.0);
        assert_eq!(tsmc.total_value, 2_100_000.0);
    }
}

/// 再平衡資料不變條件與 ETF 穿透的回歸測試。
#[cfg(test)]
mod regression {
    use super::*;
    /// 產生一筆有效且可交易的測試部位。
    fn position(symbol: &str, is_etf: bool) -> CurrentPosition {
        CurrentPosition {
            symbol: symbol.into(),
            name: symbol.into(),
            shares: 10_000,
            current_price: 100.0,
            industry: if is_etf { "ETF" } else { "半導體業" }.into(),
            is_etf,
        }
    }
    /// ETF 市值必須依成分股產業穿透，不能將 ETF 本身當成產業。
    #[test]
    fn industry_looks_through_etf_and_uses_master_classification() {
        let components = [EtfLookThroughComponent {
            stock_symbol: "2330".into(),
            name: "台積電".into(),
            weight_percent: 60.0,
            etf_symbol: "FUND".into(),
            industry: "半導體業".into(),
        }];
        let result = calculate_rebalance(
            TargetDimension::Industry,
            &[("半導體業".into(), 60.0)],
            &[position("FUND", true)],
            &components,
            &RebalanceParams::default(),
        )
        .unwrap();
        assert_eq!(result.industry_items[0].current_value, 600_000.0);
        assert_eq!(result.industry_items[0].current_weight, 60.0);
        let exposure = calculate_rebalance(
            TargetDimension::EtfLookThrough,
            &[],
            &[position("FUND", true)],
            &components,
            &RebalanceParams::default(),
        )
        .unwrap();
        assert_eq!(exposure.etf_items[0].indirect_value, 600_000.0);
    }
    /// ETF 賣出沿用回測費率，零目標也可提出全數賣出的建議。
    #[test]
    fn cash_target_sells_etf_with_etf_tax() {
        let result = calculate_rebalance(
            TargetDimension::Stock,
            &[("FUND".into(), 0.0)],
            &[position("FUND", true)],
            &[],
            &RebalanceParams::default(),
        )
        .unwrap();
        assert_eq!(result.target_cash_percent, 100.0);
        assert_eq!(result.stock_items[0].suggested_shares, 10_000);
        assert_eq!(result.stock_items[0].estimated_cost, 2425.0);
    }
    /// 非有限值、重複 key 與超出儲存精度皆不可進入計算。
    #[test]
    fn rejects_invalid_weights_and_unpriced_positions() {
        for targets in [
            vec![("A".into(), f64::NAN)],
            vec![("A".into(), 33.335)],
            vec![("A".into(), 10.0), ("A".into(), 10.0)],
            vec![(" ".into(), 0.0)],
        ] {
            assert!(
                calculate_rebalance(
                    TargetDimension::Stock,
                    &targets,
                    &[],
                    &[],
                    &RebalanceParams::default()
                )
                .is_err()
            );
        }
        let mut p = position("A", false);
        p.current_price = 0.0;
        assert!(
            calculate_rebalance(
                TargetDimension::Stock,
                &[],
                &[p],
                &[],
                &RebalanceParams::default()
            )
            .is_err()
        );
        for params in [
            RebalanceParams {
                available_cash: f64::INFINITY,
                ..Default::default()
            },
            RebalanceParams {
                tolerance_percent: f64::NAN,
                ..Default::default()
            },
            RebalanceParams {
                lot_size: 10,
                ..Default::default()
            },
        ] {
            assert!(calculate_rebalance(TargetDimension::Stock, &[], &[], &[], &params).is_err());
        }
    }
}
