use std::{collections::HashSet, time::Duration};

use crate::{
    crawler::goodinfo,
    database::{table, table::stock},
    logging, nosql,
    util::map::{vec_to_hashmap, Keyable},
};
use anyhow::Result;
use scopeguard::defer;

/// 將股息中盈餘分配率為零的數據向第三方取得數據後更新更新
pub async fn execute() -> Result<()> {
    logging::info_file_async("更新盈餘分配率開始");
    defer! {
       logging::info_file_async("更新盈餘分配率結束");
    }

    let without_payout_ratio =
        table::dividend::extension::payout_ratio_info::fetch_without_payout_ratio().await?;
    let mut unique_security_code: HashSet<String> = HashSet::new();

    for wpr in &without_payout_ratio {
        unique_security_code.insert(wpr.security_code.to_string());
    }

    let mut dividend_without_payout_ratio = vec_to_hashmap(without_payout_ratio);

    for security_code in unique_security_code {
        if stock::is_preference_shares(&security_code) {
            continue;
        }

        let cache_key = format!("goodinfo:payout_ratio:{}", security_code);
        let is_jump = nosql::redis::CLIENT.get_bool(&cache_key).await?;
        if is_jump {
            continue;
        }

        nosql::redis::CLIENT
            .set(cache_key, true, 60 * 60 * 24 * 7)
            .await?;

        let dividends_from_goodinfo = goodinfo::dividend::visit(&security_code).await?;
        for gds in dividends_from_goodinfo.values() {
            for gd in gds {
                let key = gd.key();
                if let Some(pri) = dividend_without_payout_ratio.get_mut(&key) {
                    pri.payout_ratio = gd.payout_ratio;
                    pri.payout_ratio_stock = gd.payout_ratio_stock;
                    pri.payout_ratio_cash = gd.payout_ratio_cash;

                    if let Err(why) = pri.update().await {
                        logging::error_file_async(format!("{} {:?}", key, why));
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
    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 payout_ratio::execute".to_string());

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to payout_ratio::execute because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 payout_ratio::execute".to_string());
    }
}
