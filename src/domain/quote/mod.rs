/// 報價領域之實體定義。
pub mod entity;
/// 報價領域之倉儲介面。
pub mod repository;
/// 報價領域之測試替身；僅在測試建置下編譯。
#[cfg(test)]
pub mod test_double;
