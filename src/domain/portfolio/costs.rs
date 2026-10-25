//! 回測與再平衡共用的交易費率；此處以小數比例表示。
/// 買賣手續費率，未計折扣。
pub const FEE_RATE: f64 = 0.001425;
/// 普通股賣出交易稅率。
pub const STOCK_TAX_RATE: f64 = 0.003;
/// ETF 賣出交易稅率。
pub const ETF_TAX_RATE: f64 = 0.001;
