use crate::domain::registry::entity::Stock;
use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;

/// <summary>
/// 證券主檔倉儲特徵介面 (Repository Trait)。
/// 定義對 Stock 聚合根進行持久化查詢與寫入的合約。
/// </summary>
#[async_trait]
pub trait StockRepository: Send + Sync {
    /// <summary>
    /// 依據證券代碼查詢 Stock 聚合根。
    /// </summary>
    async fn find_by_symbol(&self, symbol: &str) -> Result<Option<Stock>>;

    /// <summary>
    /// 新增或更新 Stock 聚合根至持久化儲存。
    /// </summary>
    async fn save(&self, stock: &Stock) -> Result<()>;

    /// <summary>
    /// 獲取所有目前非下市 (有效交易中) 的證券主檔。
    /// </summary>
    async fn fetch_all_active(&self) -> Result<Vec<Stock>>;

    /// <summary>
    /// 更新個股最新一季與近四季的 EPS、ROE 等財務指標。
    /// </summary>
    async fn update_eps_and_roe(&self) -> Result<()>;

    /// <summary>
    /// 取得所有每股淨值為零的非下市證券主檔。
    /// </summary>
    async fn fetch_net_asset_value_per_share_is_zero(&self) -> Result<Vec<Stock>>;

    /// <summary>
    /// 取得指定年度與季別中，缺漏財務報表的證券代號清單。
    /// </summary>
    async fn fetch_stocks_without_financial_statement(
        &self,
        year: i32,
        quarter: &str,
    ) -> Result<Vec<String>>;

    /// <summary>
    /// 將所有有效證券的權值占比重置歸零。
    /// </summary>
    async fn zeroed_out_weights(&self) -> Result<()>;

    /// <summary>
    /// 更新指定證券代號的權值占比。
    /// </summary>
    async fn update_weight(&self, stock_symbol: &str, weight: Decimal) -> Result<()>;
}
