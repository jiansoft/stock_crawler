//! # Fugle 行情採集模組
//!
//! 此模組負責透過 Fugle 官方 REST API 取得台股日內即時行情。
//!
//! ## 支援功能
//!
//! - **即時報價 (`price`)**：抓取最新成交價、漲跌與漲跌幅。
//!
//! ## 站點資訊
//!
//! - 來源域名：`api.fugle.tw`
//! - 存取方式：HTTP GET 搭配 API Key 驗證
//! - 主要端點：`/marketdata/v1.0/stock/intraday/quote/{symbol}`

/// Fugle 即時報價子模組。
pub mod price;

/// Fugle 行情 API 主機域名。
const HOST: &str = "api.fugle.tw";

/// Fugle 行情採集器。
///
/// 此結構體作為 `StockInfo` 的實作載體，
/// 將 Fugle 官方日內行情 API 包裝為統一的抓價介面。
pub struct Fugle {}
