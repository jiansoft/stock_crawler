//! 除權息填息事件重算的整合測試（臨時庫、假代號、固定 1990／1995 年資料）。

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;
use crate::infra::database;

const SYMBOL: &str = "79979FE";

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM dividend_fill_event WHERE stock_symbol = $1",
        "DELETE FROM dividend WHERE security_code = $1",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 一列 `(除息日, 現金股利, 除息前收盤, 第幾個交易日填息, 填息日, 觀察期滿日)`。
type EventRow = (
    NaiveDate,
    Decimal,
    Option<Decimal>,
    Option<i16>,
    Option<NaiveDate>,
    Option<NaiveDate>,
);

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("固定日期合法")
}

/// 1995-03-01 除息 1 元：除息前收盤 10 元，03-01 收 9、03-02 收 9.5、03-03 起收 10（第 3 個交易日填息）；
/// 03-01 起連續 125 天都有日線，第 120 筆是 06-28。同日的全年彙總列（4 元）不取；1990-07-01 的事件沒有日線，
/// 填息欄位為 NULL；02-30 的無效日期不會讓整批重算失敗。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_rebuilds_fill_events_from_dividends_and_quotes() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 execute_rebuilds_fill_events_from_dividends_and_quotes：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    cleanup(pool).await;
    for sql in [
        r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2")
           VALUES ($1, 1995, 1994, 'Q4', 1, 0, 1, '1995-03-01', '-'),
                  ($1, 1995, 1994, '', 4, 0, 4, '1995-03-01', '-'),
                  ($1, 1995, 1995, 'Q1', 1, 0, 1, '1995-02-30', '-'),
                  ($1, 1990, 1989, '', 1, 0, 1, '1990-07-01', '-')"#,
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice")
           VALUES ('1995-02-28', $1, 1995, 2, 28, 10, 10, 10, 10)"#,
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice")
           SELECT d::date, $1, EXTRACT(YEAR FROM d)::int, EXTRACT(MONTH FROM d)::int, EXTRACT(DAY FROM d)::int,
                  c, c, c, c
           FROM generate_series('1995-03-01'::date, '1995-07-03'::date, '1 day') AS d
           CROSS JOIN LATERAL (
               SELECT CASE d::date WHEN '1995-03-01' THEN 9 WHEN '1995-03-02' THEN 9.5 ELSE 10 END::numeric AS c
           ) price"#,
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("寫入測試資料");
    }

    execute().await.expect("重算填息事件");

    let rows: Vec<EventRow> = sqlx::query_as(
        "SELECT ex_date, cash_dividend, pre_ex_close, days_to_fill, fill_date, mature_date
         FROM dividend_fill_event WHERE stock_symbol = $1 ORDER BY ex_date",
    )
    .bind(SYMBOL)
    .fetch_all(pool)
    .await
    .expect("讀取填息事件");
    cleanup(pool).await;

    assert_eq!(
        rows,
        [
            (date(1990, 7, 1), dec!(1), None, None, None, None),
            (
                date(1995, 3, 1),
                dec!(1),
                Some(dec!(10)),
                Some(3),
                Some(date(1995, 3, 3)),
                Some(date(1995, 6, 28)),
            ),
        ]
    );
}
