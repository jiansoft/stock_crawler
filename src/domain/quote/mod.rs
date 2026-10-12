/// 報價領域之實體定義。
pub mod entity;
/// 官方本益比與股價淨值比（回補用型別與倉儲介面）。
pub mod official_ratio;
/// 同業比較（產業中位數與名次）。
pub mod peer;
/// 報價領域之倉儲介面。
pub mod repository;
/// 報價領域之測試替身；僅在測試建置下編譯。
#[cfg(test)]
pub mod test_double;
/// 本益比／股價淨值比歷史區間統計。
pub mod valuation_band;
