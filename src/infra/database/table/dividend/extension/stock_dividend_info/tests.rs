//! 除權息日股利資訊查詢的測試。

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
        println!("跳過 fetch_stocks_with_dividends_on_date_covers_next_year_payout：無資料庫連接");
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
