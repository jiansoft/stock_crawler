use crate::domain::portfolio::entity::{
    ReceivedDividend, ReceivedDividendItem, StockOwnershipDetail,
};
use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;

/// 持股領域之倉儲介面 (Repository Trait)。
///
/// 定義對持股明細 (`StockOwnershipDetail`) 及已領股利紀錄之讀取與持久化合約。
#[async_trait]
pub trait PortfolioRepository: Send + Sync {
    /// 依證券代號清單查詢所有「未售出」的持股明細。
    /// 若傳入 `None`，則查詢所有未售出的持股。
    async fn fetch_active_holdings(
        &self,
        security_codes: Option<Vec<String>>,
    ) -> Result<Vec<StockOwnershipDetail>>;

    /// 依序號尋找單筆持股明細。
    async fn find_holding_by_serial(&self, serial: i64) -> Result<Option<StockOwnershipDetail>>;

    /// 更新持股明細之累積已領股利數值。
    async fn update_holding_dividends(&self, holding: &StockOwnershipDetail) -> Result<()>;

    /// 儲存持股年度已領股利總計 (`ReceivedDividend`) 及其各配發宣告項目 (`ReceivedDividendItem`)。
    ///
    /// 應在同一交易 (Transaction) 內執行，並更新總計與細項明細。
    async fn save_received_dividend(
        &self,
        summary: &ReceivedDividend,
        items: &[ReceivedDividendItem],
    ) -> Result<()>;

    /// 刪除指定持股在特定年度的已領股利總計與細項明細。
    ///
    /// 應在同一交易 (Transaction) 內執行，先刪除明細細項，再刪除總計。
    async fn delete_received_dividend(&self, holding_serial: i64, year: i32) -> Result<()>;

    /// 計算指定持股之累積已領取股利總額。
    ///
    /// 回傳格式為：`(累積現金元, 累積股票股, 累積股票元, 累積總額元)`。
    async fn calculate_accumulated_dividends(
        &self,
        holding_serial: i64,
    ) -> Result<(Decimal, Decimal, Decimal, Decimal)>;
}
