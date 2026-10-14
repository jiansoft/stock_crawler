/// 報價領域之實體定義。
pub mod entity;
/// 除權息日漲跌修正（以官方開盤競價基準重算）。
pub mod ex_right;
/// 未來報酬（訊號出現後 5／20／60 個交易日的還原報酬）。
pub mod forward_return;
/// 官方本益比與股價淨值比（回補用型別與倉儲介面）。
pub mod official_ratio;
/// 同業比較（產業中位數與名次）。
pub mod peer;
/// 報價領域之倉儲介面。
pub mod repository;
/// 技術指標（均線、RSI、KD、MACD 與交叉訊號）。
pub mod technical;
/// 報價領域之測試替身；僅在測試建置下編譯。
#[cfg(test)]
pub mod test_double;
/// 本益比／股價淨值比歷史區間統計。
pub mod valuation_band;
