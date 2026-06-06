use crate::domain::registry::repository::StockRepository;
use crate::infra::database::repository::stock::PgStockRepository;
use anyhow::Result;

/// 更新興櫃股票的每股淨值
pub mod emerging;
/// 將每股淨值為零的股票嚐試從yahoo取得數據後更新
pub mod zero_value;

/// 更新興櫃股票的每股淨值，資料庫更新後會更新 SHARE.stocks
pub async fn update(stock: &crate::domain::registry::entity::Stock) -> Result<()> {
    let repo = PgStockRepository::new();
    repo.save(stock).await
}
