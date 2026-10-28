//! # 投資組合再平衡純計算函式（Portfolio Rebalance）
//!
//! 提供個股（Stock）、產業（Industry）與 ETF 穿透（EtfLookThrough）三種維度的資產配置偏離試算
//! 與具體股數／金額調整建議。本模組為純函式與無狀態計算，易於單元測試。

use anyhow::Result;

mod industry;
mod look_through;
mod stock;
mod validate;
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

    /// `etf_symbols` 的前十大成分股與各自的資料日期，供 ETF 穿透維度試算。
    /// 預設回空清單（不支援穿透的實作視為沒有成分資料）。
    async fn fetch_etf_components(&self, etf_symbols: &[String]) -> Result<Vec<DatedEtfComponent>> {
        let _ = etf_symbols;
        Ok(Vec::new())
    }
}

/// 附資料日期（`etf_holding.as_of`）的 ETF 成分股。
#[derive(Debug, Clone, PartialEq)]
pub struct DatedEtfComponent {
    /// 成分股與權重。
    pub component: EtfLookThroughComponent,
    /// 成分資料的日期。
    pub as_of: chrono::NaiveDate,
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
    validate::validate_inputs(positions, etf_components, params)?;
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

    let (stock_items, industry_items, etf_items, cash_summary) = match dimension {
        TargetDimension::Stock => {
            let (items, cash) =
                stock::rebalance_stocks(targets, positions, total_portfolio, params);
            (items, Vec::new(), Vec::new(), cash)
        }
        TargetDimension::Industry => (
            Vec::new(),
            industry::rebalance_industries(targets, positions, etf_components, total_portfolio),
            Vec::new(),
            empty_cash_summary,
        ),
        TargetDimension::EtfLookThrough => (
            Vec::new(),
            Vec::new(),
            look_through::rebalance_look_through(
                targets,
                positions,
                etf_components,
                total_portfolio,
            ),
            empty_cash_summary,
        ),
    };
    Ok(PortfolioRebalanceResult {
        dimension,
        total_equity_value: round2(total_equity),
        total_portfolio_value: round2(total_portfolio),
        target_cash_percent: round2(target_cash_percent),
        tolerance_percent: params.tolerance_percent,
        lot_size,
        stock_items,
        industry_items,
        etf_items,
        cash_summary,
        disclaimer: REBALANCE_DISCLAIMER,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod regression;
