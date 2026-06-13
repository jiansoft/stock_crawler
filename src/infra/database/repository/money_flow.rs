use crate::{
    domain::money_flow::{
        entity::MoneyFlowMemberWithPreviousDay as DomainMoneyFlowMemberWithPreviousDay,
        repository::MoneyFlowRepository,
    },
    infra::database::table::{
        daily_stock_price_stats::DailyStockPriceStats as TableDailyStockPriceStats,
        money_flow::{
            daily_money_history::DailyMoneyHistory as TableDailyMoneyHistory,
            daily_money_history_detail::DailyMoneyHistoryDetail as TableDailyMoneyHistoryDetail,
            daily_money_history_detail_more::DailyMoneyHistoryDetailMore as TableDailyMoneyHistoryDetailMore,
            daily_money_history_member::{
                DailyMoneyHistoryMember as TableDailyMoneyHistoryMember,
                DailyMoneyHistoryMemberWithPreviousTradingDay as TableDailyMoneyHistoryMemberWithPreviousTradingDay,
            },
        },
    },
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::NaiveDate;

/// PostgreSQL 實作之資金流向與帳戶市值倉儲。
///
/// 基於 PostgreSQL (SQLx) 實現 `MoneyFlowRepository` 介面，
/// 負責將跨多張市值表的複雜交易寫入行為整合於倉儲內部，以提供外部簡潔的領域合約。
#[derive(Default)]
pub struct PgMoneyFlowRepository;

impl PgMoneyFlowRepository {
    /// 建立 `PgMoneyFlowRepository` 新實例。
    pub fn new() -> Self {
        // 傳回全新的 PgMoneyFlowRepository 實例
        PgMoneyFlowRepository
    }
}

// === 實體映射實作 ===

impl From<TableDailyMoneyHistoryMemberWithPreviousTradingDay>
    for DomainMoneyFlowMemberWithPreviousDay
{
    fn from(table: TableDailyMoneyHistoryMemberWithPreviousTradingDay) -> Self {
        // 將資料庫對照表結構模型轉換為領域實體，以便應用層處理
        DomainMoneyFlowMemberWithPreviousDay {
            date: table.date,
            previous_date: table.previous_date,
            member_id: table.member_id,
            market_value: table.market_value,
            previous_market_value: table.previous_market_value,
        }
    }
}

#[async_trait]
impl MoneyFlowRepository for PgMoneyFlowRepository {
    async fn recalculate_and_save_money_flow(&self, date: NaiveDate) -> Result<()> {
        // 1. 初始化資料庫事務，確保跨表寫入的一致性
        let mut tx_option = Some(crate::infra::database::get_tx().await?);

        // 2. 寫入當日市值總覽 (扁平表)
        if let Err(why) = TableDailyMoneyHistory::upsert(date, &mut tx_option).await {
            // 若失敗則嘗試回滾 Transaction
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!("Failed to upsert daily_money_history: {:?}", why));
        }

        // 3. 寫入會員垂直總覽 (垂直表，利於擴充會員維度)
        if let Err(why) = TableDailyMoneyHistoryMember::upsert(date, &mut tx_option).await {
            // 若失敗則嘗試回滾 Transaction
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to upsert daily_money_history_member: {:?}",
                why
            ));
        }

        // 4. 先清空當日舊明細，再重建持股層級明細表
        if let Err(why) = TableDailyMoneyHistoryDetail::delete(date, &mut tx_option).await {
            // 若失敗則嘗試回滾 Transaction
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to delete daily_money_history_detail: {:?}",
                why
            ));
        }

        if let Err(why) = TableDailyMoneyHistoryDetail::upsert(date, &mut tx_option).await {
            // 若失敗則嘗試回滾 Transaction
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to upsert daily_money_history_detail: {:?}",
                why
            ));
        }

        // 5. 先清空當日舊明細，再重建交易批次層級明細表（必須在 detail 表 upsert 後執行，因有依賴關係）
        if let Err(why) = TableDailyMoneyHistoryDetailMore::delete(date, &mut tx_option).await {
            // 若失敗則嘗試回滾 Transaction
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to delete daily_money_history_detail_more: {:?}",
                why
            ));
        }

        if let Err(why) = TableDailyMoneyHistoryDetailMore::upsert(date, &mut tx_option).await {
            // 若失敗則嘗試回滾 Transaction
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to upsert daily_money_history_detail_more: {:?}",
                why
            ));
        }

        // 6. 更新當日市場全市場統計
        if let Err(why) = TableDailyStockPriceStats::upsert(date, &mut tx_option).await {
            // 若失敗則嘗試回滾 Transaction
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to upsert daily_stock_price_stats: {:?}",
                why
            ));
        }

        // 7. 所有步驟均無異常，正式提交 Transaction
        if let Some(tx) = tx_option {
            tx.commit().await?;
        }

        Ok(())
    }

    async fn fetch_member_money_history_with_previous_day(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<DomainMoneyFlowMemberWithPreviousDay>> {
        // 從資料庫加載原始對照資料
        let table_rows =
            TableDailyMoneyHistoryMember::fetch_with_previous_trading_day(date).await?;
        // 映射轉換為領域實體清單並傳回
        let domain_rows = table_rows
            .into_iter()
            .map(DomainMoneyFlowMemberWithPreviousDay::from)
            .collect();
        Ok(domain_rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::database;

    #[tokio::test]
    async fn test_money_flow_repository_flow() {
        // 載入環境變數
        dotenv::dotenv().ok();

        if database::ping().await.is_err() {
            println!("跳過 test_money_flow_repository_flow：無資料庫連接");
            return;
        }

        // 建立資金流向與市值倉儲實例
        let repo = PgMoneyFlowRepository::new();
        // 選擇有完整資料之特定測試交易日
        let test_date = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap();

        // 1. 執行 recalculate_and_save_money_flow，驗證多表交易式批次更新是否可順利執行
        let result = repo.recalculate_and_save_money_flow(test_date).await;
        assert!(
            result.is_ok(),
            "recalculate_and_save_money_flow failed: {:?}",
            result.err()
        );

        // 2. 驗證讀取會員對照資料是否正常
        let compare_data = repo
            .fetch_member_money_history_with_previous_day(test_date)
            .await;
        assert!(
            compare_data.is_ok(),
            "fetch_member_money_history_with_previous_day failed: {:?}",
            compare_data.err()
        );
        let compare_data = compare_data.unwrap();
        // 預期至少有合計列 (member_id = 0)
        assert!(!compare_data.is_empty());
        assert!(compare_data.iter().any(|r| r.member_id == 0));
    }
}
