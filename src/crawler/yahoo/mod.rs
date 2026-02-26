//! # Yahoo 財經採集模組
//!
//! 此模組專門負責從 Yahoo 財經（台灣站）抓取各類證券資料。
//!
//! ## 支援的功能
//!
//! - **即時報價 (`price`)**：抓取最新成交價、漲跌幅、開盤、最高、最低價等。
//! - **基本面資料 (`profile`)**：抓取毛利率、營益率、ROE、ROA、EPS 等財務比率。
//! - **股利政策 (`dividend`)**：抓取歷年現金股利、股票股利、除息日及發放日明細。
//!
//! ## 站點資訊
//!
//! - 來源域名：`tw.stock.yahoo.com`
//! - 抓取技術：HTTP GET 搭配 CSS Selector 解析。

/// 股利數據採集子模組
pub mod dividend;
/// 即時報價與行情採集子模組
pub mod price;
/// 財務比率與基本面資料採集子模組
pub mod profile;

/// Yahoo 財經台灣站的主機域名
const HOST: &str = "tw.stock.yahoo.com";

/// Yahoo 財經採集器
///
/// 此結構體主要作為 `StockInfo` Trait 的實作載體，提供統一的採集介面。
pub struct Yahoo {}
