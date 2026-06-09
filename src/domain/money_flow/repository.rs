use crate::domain::money_flow::entity::MoneyFlowMemberWithPreviousDay;
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 資金流向與帳戶市值領域之倉儲介面 (Repository Trait)。
///
/// 隔離資料庫交易與複雜的多表寫入細節，定義對每日市值、市值明細、會員市值總額的計算與存取合約。
#[async_trait]
pub trait MoneyFlowRepository: Send + Sync {
    /// 依指定交易日重算並交易式寫入所有市值與明細資料。
    ///
    /// 此方法必須在實作中封裝資料庫 Transaction，依序執行：
    /// 1. 更新每日市值總覽 (daily_money_history)
    /// 2. 更新會員垂直總覽 (daily_money_history_member)
    /// 3. 先清除後重建每日持股明細 (daily_money_history_detail)
    /// 4. 先清除後重建交易批次明細 (daily_money_history_detail_more)
    /// 5. 更新當日市場統計 (daily_stock_price_stats)
    /// 任一步驟失敗即會自動 Transaction Rollback，以確保多張資料表在該日期下的數據一致性。
    async fn recalculate_and_save_money_flow(&self, date: NaiveDate) -> Result<()>;

    /// 取得指定交易日與前一交易日之會員市值對照。
    ///
    /// 傳回包含合計 (member_id = 0) 及各個個別會員在當日與前一日的市值數據。
    async fn fetch_member_money_history_with_previous_day(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<MoneyFlowMemberWithPreviousDay>>;
}
