//! # Winvest 採集器
//!
//! 此模組封裝 Winvest（`winvest.tw`）的即時報價來源，並提供
//! `StockInfo` trait 所需的股價與報價查詢能力。
//!
//! 目前功能由 [`price`] 子模組提供。

/// Winvest 即時報價實作。
pub mod price;

/// Winvest 網站主機名稱。
const HOST: &str = "winvest.tw";

/// Winvest 採集器型別。
///
/// 此型別本身不持有狀態，主要作為 `StockInfo` 的實作者，
/// 讓外部可透過統一介面呼叫：
/// - `Winvest::get_stock_price`
/// - `Winvest::get_stock_quotes`
pub struct Winvest {}
