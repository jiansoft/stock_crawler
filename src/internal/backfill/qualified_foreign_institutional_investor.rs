use anyhow::Result;
use chrono::{DateTime, FixedOffset, Local};

use crate::{
    cache::SHARE, crawler::twse,
    database::table::stock::extension::qualified_foreign_institutional_investor::QualifiedForeignInstitutionalInvestor,
    logging, util::datetime::Weekend,
};

pub async fn execute() -> Result<()> {
    let now = Local::now();

    if now.is_weekend() {
        return Ok(());
    }

    tokio::try_join!(listed(now.fixed_offset()), otc())?;

    Ok(())
}

async fn listed(date_time: DateTime<FixedOffset>) -> Result<()> {
    let listed = twse::qualified_foreign_institutional_investor::listed::visit(date_time).await?;
    update(listed).await
}

async fn otc() -> Result<()> {
    let toc = twse::qualified_foreign_institutional_investor::over_the_counter::visit().await?;
    update(toc).await
}

/// 更新股票的外資持股狀況，資料庫更新後會更新 SHARE.stocks
async fn update(qfiis: Vec<QualifiedForeignInstitutionalInvestor>) -> Result<()> {
    for qfii in qfiis {
        // 嘗試讀取stocks_cache
        let stock_cache = SHARE.get_stock(&qfii.stock_symbol).await;
        match stock_cache {
            None => {
                continue;
            }
            Some(stock) => {
                if stock.issued_share == qfii.issued_share
                    && stock.qfii_shares_held == qfii.qfii_shares_held
                    && stock.qfii_share_holding_percentage == qfii.qfii_share_holding_percentage
                {
                    continue;
                }
            }
        }

        // 更新qfii
        match qfii.update().await {
            Ok(_) => {
                // 嘗試更新stocks_cache
                if let Ok(mut stocks_cache) = SHARE.stocks.write() {
                    if let Some(stock_cache) = stocks_cache.get_mut(&qfii.stock_symbol) {
                        stock_cache.qfii_shares_held = qfii.qfii_shares_held;
                        stock_cache.issued_share = qfii.issued_share;
                        stock_cache.qfii_share_holding_percentage =
                            qfii.qfii_share_holding_percentage;
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("{:?}", why));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};

    use super::*;

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
