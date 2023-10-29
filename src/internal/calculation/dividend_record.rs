use anyhow::{anyhow, Result};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    internal::database::{
        self,
        table::{
            dividend, dividend_record_detail::DividendRecordDetail, dividend_record_detail_more,
            stock_ownership_details,
        },
    },
    logging,
};

/// 計算指定年份領取的股利
pub async fn execute(year: i32, security_codes: Option<Vec<String>>) {
    logging::info_file_async("計算指定年份領取的股利開始".to_string());

    match stock_ownership_details::StockOwnershipDetail::fetch(security_codes).await {
        Ok(inventories) => {
            if !inventories.is_empty() {
                let tasks = inventories
                    .into_iter()
                    .map(|sod| calculate_dividend(sod, year))
                    .collect::<Vec<_>>();
                let results = futures::future::join_all(tasks).await;
                results
                    .into_iter()
                    .filter_map(|r| r.err())
                    .for_each(|e| logging::error_file_async(format!("{:?}", e)));
            }
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to execute StockOwnershipDetail::fetch because {:?}",
                why
            ));
        }
    }

    logging::info_file_async("計算指定年份領取的股利結束".to_string());
}

/// 計算股票於該年度可以領取的股利
async fn calculate_dividend(
    mut sod: stock_ownership_details::StockOwnershipDetail,
    year: i32,
) -> Result<()> {
    //計算股票於該年度可以領取的股利
    let mut d = dividend::Dividend::new();
    d.security_code = sod.security_code.to_string();
    d.year = year;
    let dividend_sum = d
        .fetch_yearly_dividends_sum_by_date(sod.created_time)
        .await?;

    let number_of_shares_held = Decimal::new(sod.share_quantity, 0);
    let dividend_cash = dividend_sum.0 * number_of_shares_held;
    let dividend_stock = dividend_sum.1 * number_of_shares_held / dec!(10);
    let dividend_stock_money = dividend_sum.1 * number_of_shares_held;
    let dividend_total = dividend_sum.2 * number_of_shares_held;
    let mut drd = DividendRecordDetail::new(
        sod.serial,
        year,
        dividend_cash,
        dividend_stock,
        dividend_stock_money,
        dividend_total,
    );

    let mut tx_option = database::get_tx().await.ok();
    //更新股利領取記錄
    let dividend_record_detail_serial = match drd.upsert(&mut tx_option).await {
        Ok(serial) => serial,
        Err(why) => {
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to execute upsert query for dividend_record_detail because {:?}",
                why
            ));
        }
    };

    let dividends = dividend::Dividend::fetch_dividends_summary_by_date(
        &d.security_code,
        d.year,
        sod.created_time,
    )
    .await?;

    for dividend in dividends {
        //寫入領取細節表
        let dividend_cash = dividend.cash_dividend * number_of_shares_held;
        let dividend_stock = dividend.stock_dividend * number_of_shares_held / Decimal::new(10, 0);
        let dividend_stock_money = dividend.stock_dividend * number_of_shares_held;
        let dividend_total = dividend.sum * number_of_shares_held;

        let mut rdrm = dividend_record_detail_more::DividendRecordDetailMore::new(
            sod.serial,
            dividend_record_detail_serial,
            dividend.serial,
            dividend_cash,
            dividend_stock,
            dividend_stock_money,
            dividend_total,
        );

        if let Err(why) = rdrm.upsert(&mut tx_option).await {
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to execute upsert query for dividend_record_detail_more because {:?}",
                why
            ));
        }

        // 計算指定股票其累積的領取股利
        let cumulate_dividend = match drd.fetch_cumulate_dividend(&mut tx_option).await {
            Ok(cd) => cd,
            Err(why) => {
                if let Some(tx) = tx_option {
                    tx.rollback().await?;
                }
                return Err(anyhow!(
                    "Failed to execute calculate_cumulate_dividend because {:?}",
                    why
                ));
            }
        };

        sod.cumulate_dividends_cash = cumulate_dividend.cash;
        sod.cumulate_dividends_stock_money = cumulate_dividend.stock_money;
        sod.cumulate_dividends_stock = cumulate_dividend.stock;
        sod.cumulate_dividends_total = cumulate_dividend.total;

        if let Err(why) = sod.update_cumulate_dividends(&mut tx_option).await {
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to execute update_cumulate_dividends because {:?}",
                why
            ));
        }
    }

    if let Some(tx) = tx_option {
        tx.commit().await?;
    }

    Ok(())
}

/*/// 計算指定年份領取的股利
pub async fn calculate(year: i32) {
    logging::info_file_async("計算指定年份領取的股利開始".to_string());
    // 先取得庫存股票
    match table::inventory::fetch().await {
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
    use chrono::{Local, TimeZone};

    use crate::internal::cache::SHARE;
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 calculate".to_string());
        for i in 2023..2024 {
            execute(i, None).await;
            logging::debug_file_async(format!("calculate({}) 完成", i));
        }
        logging::debug_file_async("結束 calculate".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_calculate_dividend() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 calculate_dividend".to_string());
        let mut sod = stock_ownership_details::StockOwnershipDetail::new();
        sod.serial = 102;
        sod.security_code = "2882".to_string();
        sod.member_id = 2;
        sod.share_quantity = 300;
        sod.created_time = Local.with_ymd_and_hms(2023, 4, 9, 0, 0, 0).unwrap();
        match calculate_dividend(sod, 2023).await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to calculate_dividend because {:?}",
                    why
                ));
            }
        }
        logging::debug_file_async("結束 calculate_dividend".to_string());
    }
}
