use crate::{internal::database::model, logging};

/// 計算指定年份領取的股利
pub async fn calculate(year: i32, security_codes: Option<Vec<String>>) {
    logging::info_file_async("計算指定年份領取的股利開始".to_string());

    if let Ok(inventories) = model::stock_ownership_details::fetch(security_codes).await {
        let tasks = inventories
            .into_iter()
            .map(|mut item| async move {
                //計算今年領取的股利，如果股利並非零時將數據更新到 dividend_record_detail 表
                let drd = item
                    .calculate_dividend_and_upsert(year)
                    .await
                    .map_err(|e| {
                        format!("Failed to calculate_dividend_and_upsert because {:?}", e)
                    })?;

                // 計算指定股票其累積的領取股利
                let cumulate_dividend = drd.calculate_cumulate_dividend().await.map_err(|e| {
                    format!("Failed to calculate_cumulate_dividend because {:?}", e)
                })?;

                let (cash, stock_money, stock, total) = cumulate_dividend;
                item.cumulate_dividends_cash = cash;
                item.cumulate_dividends_stock_money = stock_money;
                item.cumulate_dividends_stock = stock;
                item.cumulate_dividends_total = total;
                item.update_cumulate_dividends()
                    .await
                    .map_err(|e| format!("Failed to update_cumulate_dividends because {:?}", e))
            })
            .collect::<Vec<_>>();
        let results = futures::future::join_all(tasks).await;
        results
            .into_iter()
            .filter_map(|r| r.err())
            .for_each(logging::error_file_async);
    } else {
        logging::error_file_async("Failed to inventory::fetch".to_string());
    }

    logging::info_file_async("計算指定年份領取的股利結束".to_string());
}

/*/// 計算指定年份領取的股利
pub async fn calculate(year: i32) {
    logging::info_file_async("計算指定年份領取的股利開始".to_string());
    // 先取得庫存股票
    match model::inventory::fetch().await {
        Ok(mut inventories) => {
            for item in inventories.iter_mut() {
                match item.calculate_dividend(year).await {
                    Ok(drd) => match drd.calculate_cumulate_dividend().await {
                        Ok(cumulate_dividend) => {
                            let (cash, stock_money, stock, total) = cumulate_dividend;
                            item.cumulate_cash = cash;
                            item.cumulate_stock_money = stock_money;
                            item.cumulate_stock = stock;
                            item.cumulate_total = total;
                            if let Err(why) = item.update_cumulate_dividends().await {
                                logging::error_file_async(format!(
                                    "Failed to update_cumulate_dividends because {:?}",
                                    why
                                ));
                            }
                        }
                        Err(why) => {
                            logging::error_file_async(format!(
                                "Failed to calculate_cumulate_dividend because {:?}",
                                why
                            ));
                        }
                    },
                    Err(why) => {
                        logging::error_file_async(format!(
                            "Failed to calculate_dividend because {:?}",
                            why
                        ));
                    }
                };
            }
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to inventory::fetch because {:?}", why));
        }
    }

    logging::info_file_async("計算指定年份領取的股利結束".to_string());
}*/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 calculate".to_string());
        for i in 2014..2024 {
            calculate(i, None).await;
        }
        logging::info_file_async("結束 calculate".to_string());
    }
}
