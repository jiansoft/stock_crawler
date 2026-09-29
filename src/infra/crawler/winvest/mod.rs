//! # Winvest 採集器
//!
//! 此模組封裝 Winvest（`winvest.tw`）的報價來源（2026-09 改版後只提供日 K），並提供
//! `StockInfo` trait 所需的股價與報價查詢能力。
//!
//! 目前功能由 [`price`] 子模組提供。

/// Winvest 報價實作。
pub mod price;
/// Winvest antiforgery 工作階段（cookie 與 token）。
mod session;

/// Winvest 網站主機名稱。
const HOST: &str = "winvest.tw";

/// Winvest 採集器型別。
///
/// 此型別本身不持有狀態，主要作為 `StockInfo` 的實作者，
/// 讓外部可透過統一介面呼叫：
/// - `Winvest::get_stock_price`
/// - `Winvest::get_stock_quotes`
pub struct Winvest {}
