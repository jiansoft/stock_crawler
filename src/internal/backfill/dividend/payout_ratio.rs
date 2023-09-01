use std::{collections::HashSet, thread, time::Duration};

use anyhow::Result;

use crate::internal::{
    crawler::goodinfo,
    database::table,
    logging,
    util::map::{vec_to_hashmap, Keyable},
};

/// 將股息中盈餘分配率為零的數據向第三方取得數據後更新更新
pub async fn execute() -> Result<()> {
    let without_payout_ratio =
        table::dividend::extension::payout_ratio_info::fetch_without_payout_ratio().await?;
    let mut unique_security_code: HashSet<String> = HashSet::new();

    for wpr in &without_payout_ratio {
        unique_security_code.insert(wpr.security_code.to_string());
    }

    let mut dividend_without_payout_ratio = vec_to_hashmap(without_payout_ratio);

    for security_code in unique_security_code {
        let dividends_from_goodinfo = goodinfo::dividend::visit(&security_code).await?;
        for gds in dividends_from_goodinfo.values() {
            for gd in gds {
                let key = gd.key();
                match dividend_without_payout_ratio.get_mut(&key) {
                    None => {
                        logging::debug_file_async(format!("key:{} 查無數據", key));
                    }
                    Some(pri) => {
                        pri.payout_ratio = gd.payout_ratio;
                        pri.payout_ratio_stock = gd.payout_ratio_stock;
                        pri.payout_ratio_cash = gd.payout_ratio_cash;

                        if let Err(why) = pri.update().await {
                            logging::error_file_async(format!("{} {:?}", key, why));
                        }
                    }
                }
            }
        }

        thread::sleep(Duration::from_secs(90));
    }

    Ok(())
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
        logging::debug_file_async("開始 payout_ratio::update".to_string());

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to payout_ratio::update because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 payout_ratio::update".to_string());
    }
}
