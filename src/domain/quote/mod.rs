/// 盤後資料（大盤指數、籌碼、外資持股）的筆數查詢，供收盤後完整性檢查使用。
pub mod after_hours;
/// 報價領域之實體定義。
pub mod entity;
/// 除權息日漲跌修正（以官方開盤競價基準重算）。
pub mod ex_right;
/// 未來報酬（訊號出現後 5／20／60／120／240 個交易日的還原報酬）。
pub mod forward_return;
// 官方本益比與股價淨值比（回補用型別與倉儲介面）。
pub mod official_ratio;
/// 同業比較（產業中位數與名次）。
pub mod peer;
// 相對強度（近 60／120／240 日加權報酬在上市櫃全體的百分位分數）。
pub mod relative_strength;
/// 報價領域之倉儲介面。
pub mod repository;
// 技術指標（均線、RSI、KD、MACD 與交叉訊號）。
pub mod technical;
/// 報價領域之測試替身；僅在測試建置下編譯。
#[cfg(test)]
pub mod test_double;
/// 本益比／股價淨值比歷史區間統計。
pub mod valuation_band;
