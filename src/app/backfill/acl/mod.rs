//! # 防腐層 (Anti-Corruption Layer)
//!
//! 用於隔離外部爬蟲資料結構（Crawler DTO）與應用層/領域層之業務邏輯命令或實體。

pub mod dividend;
pub mod financial;
pub mod index;
pub mod misc;
pub mod quote;
pub mod revenue;
pub mod stock;

pub use dividend::{DividendAclMapper, YahooDividendAclMapper};
pub use financial::{FinancialStatementAclMapper, NetAssetValueAclMapper};
pub use index::IndexAclMapper;
pub use misc::{QfiiAclMapper, SaveStockWeightCommand, StockWeightAclMapper, UpdateQfiiCommand};
pub use quote::QuoteAclMapper;
pub use revenue::{RevenueAclMapper, UpdateRevenueCommand};
pub use stock::{DelistedCompanyAclMapper, EtfAclMapper, IsinAclMapper, RegisterStockCommand};
