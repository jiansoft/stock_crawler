use crate::{
    core::logging, core::util::datetime::Weekend, domain::registry::repository::StockRepository,
    infra::crawler::twse, infra::database::repository::stock::PgStockRepository,
    infra::database::table::stock::extension::qualified_foreign_institutional_investor::QualifiedForeignInstitutionalInvestor,
};
use anyhow::Result;
use chrono::{DateTime, FixedOffset, Local};
use scopeguard::defer;

/// 回補上市與上櫃外資持股狀況。
pub async fn execute() -> Result<()> {
    let now = Local::now();

    if now.is_weekend() {
        return Ok(());
    }
    logging::info_file_async("更新台股外資持股狀態開始");
    defer! {
       logging::info_file_async("更新台股外資持股狀態結束");
    }

    tokio::try_join!(listed(now.fixed_offset()), otc())?;

    Ok(())
}

async fn listed(date_time: DateTime<FixedOffset>) -> Result<()> {
    let listed = twse::qualified_foreign_institutional_investor::listed::visit(date_time).await?;
    update(listed).await
}

/// 回補上櫃外資持股資料。
async fn otc() -> Result<()> {
    let toc = twse::qualified_foreign_institutional_investor::over_the_counter::visit().await?;
    update(toc).await
}

/// 更新股票的外資持股狀況，資料庫更新後會更新 SHARE.stocks
async fn update(qfiis: Vec<QualifiedForeignInstitutionalInvestor>) -> Result<()> {
    let repo = PgStockRepository::new();

    for qfii in qfiis {
        // 嘗試讀取 Stock 聚合根
        let stock_opt = repo.find_by_symbol(&qfii.stock_symbol).await?;
        if let Some(mut stock) = stock_opt {
            if stock.issued_share() == qfii.issued_share
                && stock.qfii_shares_held() == qfii.qfii_shares_held
                && stock.qfii_share_holding_percentage() == qfii.qfii_share_holding_percentage
            {
                continue;
            }

            // 使用領域模型更新狀態
            stock.update_qfii(qfii.qfii_shares_held, qfii.qfii_share_holding_percentage);
            stock.update_issued_shares(qfii.issued_share);

            // 儲存 Stock 聚合根，同時更新 DB 與快取
            if let Err(why) = repo.save(&stock).await {
                logging::error_file_async(format!(
                    "Failed to save stock QFII updates for {} because {:?}",
                    qfii.stock_symbol, why
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{core::logging, infra::cache::SHARE};

    use super::*;

    /// 驗證外資持股回補流程。
    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
