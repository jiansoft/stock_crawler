use crate::internal::database::{model, DB};
use crate::logging;
use anyhow::Result;
use chrono::Local;
use rust_decimal::Decimal;
use sqlx::postgres::PgRow;
use sqlx::Row;

/// 計算指定年份領取的股利
pub async fn calculate(year: i32) {
    // 先取得庫存股票
    match model::inventory::fetch().await {
        Ok(inventories) => {
            for mut inventory in inventories {
                match calculate_dividend(year, &inventory).await {
                    Ok(_) => match calculate_cumulate_dividend(&inventory).await {
                        Ok(cumulate_dividend) => {
                            let (cash, stock_money, stock, total) = cumulate_dividend;
                            inventory.cumulate_dividends_cash = cash;
                            inventory.cumulate_dividends_stock_money = stock_money;
                            inventory.cumulate_dividends_stock = stock;
                            inventory.cumulate_dividends_total = total;
                            if let Err(why) = inventory.update_cumulate_dividends().await {
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
}

/// 計算指定年份與股票其領取的股利
async fn calculate_dividend(year: i32, e: &model::inventory::Entity) -> Result<()> {
    logging::info_file_async(format!("stock:{:#?}", e));
    //計算股票於該年度可以領取的股利

    let dividend = sqlx::query(
        r#"
select
    COALESCE(sum(cash_dividend),0) as cash,
    COALESCE(sum(stock_dividend),0) as stock,
    COALESCE(sum(sum),0) as sum
from dividend
where security_code = $1
    and year = $2
    and ("ex-dividend_date1" >= $3 or "ex-dividend_date2" >= $4)
    and ("ex-dividend_date1" <= $5 );
        "#,
    )
    .bind(&e.security_code)
    .bind(year)
    .bind(e.create_time.format("%Y-%m-%d 00:00:00").to_string())
    .bind(e.create_time.format("%Y-%m-%d 00:00:00").to_string())
        .bind(Local::now().format("%Y-%m-%d 00:00:00").to_string())
    .try_map(|row: PgRow| {
        let cash: Decimal = row.try_get("cash")?;
        let stock: Decimal = row.try_get("stock")?;
        let sum: Decimal = row.try_get("sum")?;
        Ok((cash, stock, sum))
    })
    .fetch_one(&DB.pool)
    .await?;

    /*
    某公司股價100元配現金0.7元、配股3.6元(以一張為例)
    現金股利＝1張ｘ1000股x股利0.7元=700元
    股票股利＝1張x1000股x股利0.36=360股
    (股票股利須除以發行面額10元)
    20048 *(0.5/10)
    */

    let number_of_shares_held = Decimal::new(e.number_of_shares_held, 0);
    let dividend_cash = dividend.0 * number_of_shares_held;
    let dividend_stock = dividend.1 * number_of_shares_held / Decimal::new(10, 0);
    let dividend_stock_money = dividend.1 * number_of_shares_held;
    let dividend_total = dividend.2 * number_of_shares_held;

    if dividend_total != Decimal::ZERO {
        let drd = model::dividend_record_detail::Entity::new(
            e.serial,
            year,
            dividend_cash,
            dividend_stock,
            dividend_stock_money,
            dividend_total,
        );

        drd.upsert().await?;
    }

    Ok(())
}

/// 計算指定股票其累積的領取股利
async fn calculate_cumulate_dividend(
    e: &model::inventory::Entity,
) -> Result<(Decimal, Decimal, Decimal, Decimal)> {
    let dividend = sqlx::query(
        r#"
select COALESCE(sum(cash), 0)        as cash,
       COALESCE(sum(stock_money), 0) as stock_money,
       COALESCE(sum(stock), 0)       as stock,
       COALESCE(sum(total), 0)       as total
from dividend_record_detail
where favorite_id = $1;
        "#,
    )
    .bind(e.serial)
    .try_map(|row: PgRow| {
        let cash: Decimal = row.try_get("cash")?;
        let stock_money: Decimal = row.try_get("stock_money")?;
        let stock: Decimal = row.try_get("stock")?;
        let total: Decimal = row.try_get("total")?;
        Ok((cash, stock_money, stock, total))
    })
    .fetch_one(&DB.pool)
    .await?;

    Ok(dividend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 calculate".to_string());
        for i in 2014..2024 {
            calculate(i).await;
        }
        logging::info_file_async("結束 calculate".to_string());
    }
}
