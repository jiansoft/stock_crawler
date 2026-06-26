use crate::domain::money_flow::repository::MoneyFlowRepository;
use crate::infra::database::repository::money_flow::PgMoneyFlowRepository;
use anyhow::Result;
use chrono::NaiveDate;

/// 計算並重建指定交易日的帳戶市值相關資料。
///
/// 這個方法會在台股收盤匯總流程
/// (`event::taiwan_stock::closing::aggregate`) 中被呼叫。
///
/// 重構後，本方法已完全 DDD 化，所有資料庫 Transaction 控制與多張市值表的重建寫入
/// 皆已被內聚封裝至 [`PgMoneyFlowRepository`]，此處僅呼叫倉儲合約。
///
/// # Errors
/// 當倉儲內部的任何寫入步驟失敗時，會自動 Rollback 並回傳錯誤。
pub async fn calculate_money_history(date: NaiveDate) -> Result<()> {
    // 實例化資金流向與帳戶市值倉儲
    let repo = PgMoneyFlowRepository::new();
    // 呼叫倉儲提供的交易式重算與存檔合約
    repo.recalculate_and_save_money_flow(date).await
}

#[cfg(test)]
mod tests {
    use crate::{core::logging, infra::cache::SHARE};

    use super::*;

    #[tokio::test]
    async fn test_calculate_money_history() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 calculate_money_history");
        let current_date = NaiveDate::parse_from_str("2026-04-20", "%Y-%m-%d").unwrap();
        match calculate_money_history(current_date).await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to calculate_money_history because {:?}", why);
            }
        }
        tracing::debug!("結束 calculate_money_history");
    }
}
