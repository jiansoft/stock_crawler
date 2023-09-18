use anyhow::Result;
use chrono::Local;
use sqlx::postgres::PgQueryResult;

use crate::internal::{
    cache::SHARE, crawler::twse,
    database::table::stock::extension::qualified_foreign_institutional_investor::QualifiedForeignInstitutionalInvestor,
    logging, util::datetime::Weekend,
};

pub async fn execute() -> Result<()> {
    let now = Local::now();

    if now.is_weekend() {
        return Ok(());
    }

    let listed = twse::qualified_foreign_institutional_investor::listed::visit(now.fixed_offset());
    let otc = twse::qualified_foreign_institutional_investor::over_the_counter::visit();
    let res = tokio::try_join!(listed, otc)?;
    let mut qfiis: Vec<QualifiedForeignInstitutionalInvestor> = Vec::with_capacity(2048);

    qfiis.extend(res.0);
    qfiis.extend(res.1);

    logging::info_file_async("Up-to-date stocks data from listed and OTC sources.".to_string());

    for qfii in qfiis {
        if let Err(why) = update(&qfii).await {
            logging::error_file_async(format!("{:?}", why));
        }
    }

    Ok(())
}

/// 更新股票的外資持股狀況，資料庫更新後會更新 SHARE.stocks
async fn update(qfii: &QualifiedForeignInstitutionalInvestor) -> Result<PgQueryResult> {
    // 嘗試讀取stocks_cache
    match SHARE.stocks.read() {
        Ok(stocks_cache) => match stocks_cache.get(&qfii.stock_symbol) {
            Some(stock_cache) => {
                if stock_cache.issued_share == qfii.issued_share
                    && stock_cache.qfii_shares_held == qfii.qfii_shares_held
                    && stock_cache.qfii_share_holding_percentage
                        == qfii.qfii_share_holding_percentage
                {
                    return Ok(PgQueryResult::default());
                }
            }
            None => {
                return Ok(PgQueryResult::default());
            }
        },
        Err(_) => {
            return Ok(PgQueryResult::default());
        }
    }

    // 更新qfii
    let r = qfii.update().await?;

    // 嘗試更新stocks_cache
    if let Ok(mut stocks_cache) = SHARE.stocks.write() {
        if let Some(stock_cache) = stocks_cache.get_mut(&qfii.stock_symbol) {
            stock_cache.qfii_shares_held = qfii.qfii_shares_held;
            stock_cache.issued_share = qfii.issued_share;
            stock_cache.qfii_share_holding_percentage = qfii.qfii_share_holding_percentage;
        }
    }

    Ok(r)
}

#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
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
