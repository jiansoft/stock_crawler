use std::{collections::HashSet, time::Duration};

use crate::{
    app::backfill::acl::DividendAclMapper,
    core::util::map::{Keyable, vec_to_hashmap},
    domain::dividend::repository::DividendRepository,
    domain::registry::entity::StockSymbol,
    infra::crawler::goodinfo,
    infra::database::repository::dividend::PgDividendRepository,
};
use anyhow::Result;
use scopeguard::defer;

/// <summary>
/// 將股息中盈餘分配率為零的數據向第三方取得數據後更新更新。
/// </summary>
pub async fn execute() -> Result<()> {
    tracing::info!("更新盈餘分配率開始");
    defer! {
       tracing::info!("更新盈餘分配率結束");
    }

    let dividend_repo = PgDividendRepository::new();
    let without_payout_ratio = dividend_repo.fetch_without_payout_ratio().await?;
    let mut unique_security_code: HashSet<String> = HashSet::new();

    for wpr in &without_payout_ratio {
        unique_security_code.insert(wpr.security_code.to_string());
    }

    let mut dividend_without_payout_ratio = vec_to_hashmap(without_payout_ratio);

    for security_code in unique_security_code {
        // 使用領域 StockSymbol 值物件判斷是否為特別股，避免與 Table 層直接耦合
        if StockSymbol(security_code.clone()).is_preference() {
            continue;
        }

        let cache_key = format!("goodinfo:payout_ratio:{}", security_code);
        let is_jump = crate::infra::nosql::redis::CLIENT
            .get_bool(&cache_key)
            .await?;
        if is_jump {
            continue;
        }

        crate::infra::nosql::redis::CLIENT
            .set(cache_key, true, 60 * 60 * 24 * 2)
            .await?;

        let dividends_from_goodinfo = goodinfo::dividend::visit(&security_code).await?;
        for (_, gds) in dividends_from_goodinfo {
            for gd in gds {
                let key = gd.key();
                if let Some(pri) = dividend_without_payout_ratio.get_mut(&key) {
                    let cmd = DividendAclMapper::from_dto(pri.serial, &gd);
                    let updated_pri = DividendAclMapper::update_payout_ratio_entity(pri, &cmd);

                    match dividend_repo.save(&updated_pri).await {
                        Ok(_) => {
                            tracing::info!(
                                "更新盈餘分配率成功: security_code={}, year_of_dividend={}, quarter={}, payout_ratio_cash={}, payout_ratio_stock={}, payout_ratio={}",
                                updated_pri.security_code,
                                updated_pri.year_of_dividend,
                                updated_pri.quarter,
                                updated_pri.payout_ratio_cash,
                                updated_pri.payout_ratio_stock,
                                updated_pri.payout_ratio
                            );
                            *pri = updated_pri;
                        }
                        Err(why) => {
                            tracing::error!("{} {:?}", key, why);
                        }
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(90)).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{core::logging, infra::cache::SHARE};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 payout_ratio::execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to payout_ratio::execute because {:?}", why);
            }
        }

        tracing::debug!("結束 payout_ratio::execute");
    }
}
