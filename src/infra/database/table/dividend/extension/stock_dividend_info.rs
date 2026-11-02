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
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    const SYMBOL: &str = "79979ND";

    async fn cleanup(pool: &sqlx::PgPool) {
        for sql in [
            "DELETE FROM dividend WHERE security_code = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
        }
    }

    /// 12 月除息、隔年發放（year 是下一年）的股利也要查得到；同一天除權的旗標分開；
    /// 其他日期的列不回傳。假代號的列結束時清除。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn fetch_stocks_with_dividends_on_date_covers_next_year_payout() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!(
                "跳過 fetch_stocks_with_dividends_on_date_covers_next_year_payout：無資料庫連接"
            );
            return;
        }
        cleanup(pool).await;
        sqlx::query(r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing") VALUES ($1, '除息年度測試', $1, 2, 1, false)"#)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("種入股票");
        for (year, quarter, cash, stock, ex_cash, ex_stock) in [
            // 2024-12-10 除息、2025 年發放：year 是 2025。
            (2025, "Q2", dec!(4.5), Decimal::ZERO, "2024-12-10", "-"),
            // 同年度的另一期，除息日不同。
            (2025, "Q3", dec!(4.5), Decimal::ZERO, "2025-03-18", "-"),
            // 同一天只除權。
            (2024, "", Decimal::ZERO, dec!(0.5), "-", "2024-12-10"),
        ] {
            sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, $2, $2, $3, $4, $5, $4 + $5, $6, $7, '-', '-')"#)
                .bind(SYMBOL)
                .bind(year)
                .bind(quarter)
                .bind(cash)
                .bind(stock)
                .bind(ex_cash)
                .bind(ex_stock)
                .execute(pool)
                .await
                .expect("種入股利");
        }

        let date = NaiveDate::from_ymd_opt(2024, 12, 10).expect("合法日期");
        let result = fetch_stocks_with_dividends_on_date(date).await;
        cleanup(pool).await;
        let mut rows: Vec<StockDividendInfo> = result
            .expect("查詢應成功")
            .into_iter()
            .filter(|row| row.stock_symbol == SYMBOL)
            .collect();
        rows.sort_by(|a, b| a.quarter.cmp(&b.quarter));
        let summary: Vec<(String, Decimal, Decimal, bool, bool)> = rows
            .into_iter()
            .map(|row| {
                (
                    row.quarter,
                    row.cash_dividend,
                    row.stock_dividend,
                    row.is_cash_ex_dividend_on_date,
                    row.is_stock_ex_dividend_on_date,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (String::new(), Decimal::ZERO, dec!(0.5), false, true),
                ("Q2".to_owned(), dec!(4.5), Decimal::ZERO, true, false),
            ],
            "隔年發放的 Q2 與同日除權都要列入，Q3（另一天）不列"
        );
    }
}
