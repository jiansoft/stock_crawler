use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use sqlx::FromRow;

use crate::infra::database;

/// 指定日期有除權或除息事件的股票資料。
///
/// 此結構同時提供：
/// 1. 第一則 Telegram 清單所需的展示欄位。
/// 2. 第二則「持股預估股利」訊息所需的原始股利數值與事件旗標。
#[derive(FromRow, Debug, Clone)]
pub struct StockDividendInfo {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 股票產業分類編號。
    pub stock_industry_id: i32,
    /// 現金股利（元）。
    pub cash_dividend: Decimal,
    /// 股票股利（股）。
    pub stock_dividend: Decimal,
    /// 股利合計（元）。
    pub sum: Decimal,
    /// 參考收盤價。
    pub closing_price: Decimal,
    /// 總殖利率（%）。
    pub dividend_yield: Decimal,
    /// 現金殖利率（%）。
    pub cash_dividend_yield: Decimal,
    /// 是否於查詢日期進行除息。
    pub is_cash_ex_dividend_on_date: bool,
    /// 是否於查詢日期進行除權。
    pub is_stock_ex_dividend_on_date: bool,
    /// 股利期別（`M01`～`M12` 月配、`Q1`～`Q4` 季配、`H1`／`H2` 半年配，其餘為年配）。
    pub quarter: String,
}

/// 取得指定日期為除權或除息日的股票。
///
/// 這裡保留 `cash_dividend` / `stock_dividend` / `sum` 的原始精度，
/// 避免先在 SQL 四捨五入後又乘上持股股數，造成預估股利累積誤差。
///
/// `dividend.year` 是發放年度，除權息日可能在前一年（12 月除息、隔年發放），所以年度條件涵蓋
/// 除權息日當年與下一年；實際比對以除權息日字串為準。
pub async fn fetch_stocks_with_dividends_on_date(
    date: NaiveDate,
) -> Result<Vec<StockDividendInfo>> {
    let sql = r#"
SELECT
       s.stock_symbol,
       s."Name"                                 AS name,
       s.stock_industry_id,
       d.cash_dividend,
       d.stock_dividend,
       d.sum,
       COALESCE(ROUND(ldq.closing_price, 2), 0) as closing_price,
       CASE
           WHEN ldq.closing_price IS NULL THEN 0
           ELSE ROUND((d.sum / ldq.closing_price) * 100, 2)
           END                                  AS dividend_yield,
       CASE
           WHEN ldq.closing_price IS NULL THEN 0
           ELSE ROUND((d.cash_dividend / ldq.closing_price) * 100, 2)
           END                                  AS cash_dividend_yield,
       d."ex-dividend_date1" = $2               AS is_cash_ex_dividend_on_date,
       d."ex-dividend_date2" = $2               AS is_stock_ex_dividend_on_date,
       d.quarter
FROM
    dividend AS d
INNER JOIN
      stocks AS s ON s.stock_symbol = d.security_code
LEFT JOIN 
last_daily_quotes AS ldq on s.stock_symbol = ldq.stock_symbol      
WHERE
    -- year 是發放年度：12 月除息、隔年 1 月發放（台積電季配）的 year 是除息日的下一年。
    d."year" BETWEEN $1 AND $1 + 1
    AND (d."ex-dividend_date1" = $2 OR d."ex-dividend_date2" = $2);
"#;

    sqlx::query_as::<_, StockDividendInfo>(sql)
        .bind(date.year())
        .bind(date.format("%Y-%m-%d").to_string())
        .fetch_all(database::get_connection())
        .await
        .context(format!(
            "Failed to fetch_stocks_with_dividends_on_date({}) from database",
            date
        ))
}

#[cfg(test)]
mod tests;
