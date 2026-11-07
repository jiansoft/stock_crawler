//! 月營收回補的測試。

use std::time::Duration;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;
use crate::infra::database;

/// 月中、月初與跨年都回到上個月最後一分鐘（台北時間）。
#[test]
fn previous_month_end_is_last_minute_of_previous_month_in_taipei() {
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("日期應合法");
    for (today, expected) in [
        (day(2026, 10, 11), "2026-09-30T23:59:00+08:00"),
        (day(2026, 10, 1), "2026-09-30T23:59:00+08:00"),
        (day(2026, 3, 31), "2026-02-28T23:59:00+08:00"),
        (day(2026, 1, 5), "2025-12-31T23:59:00+08:00"),
    ] {
        assert_eq!(previous_month_end(today).to_rfc3339(), expected, "{today}");
    }
}

/// 假代號：A 有當月日線，B 沒有。
const SYMBOL_A: &str = "79979RV";
const SYMBOL_B: &str = "79979RW";

async fn cleanup() {
    let pool = database::get_connection();
    for symbol in [SYMBOL_A, SYMBOL_B] {
        for sql in [
            r#"DELETE FROM "Revenue" WHERE stock_symbol = $1"#,
            "DELETE FROM revenue_last_date WHERE stock_symbol = $1",
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(symbol).execute(pool).await;
        }
    }
}

fn command(symbol: &str, monthly: Decimal) -> UpdateRevenueCommand {
    UpdateRevenueCommand {
        symbol: symbol.to_owned(),
        monthly,
        last_month: dec!(900),
        last_year_this_month: dec!(800),
        monthly_accumulated: monthly,
        last_year_monthly_accumulated: dec!(800),
        compared_with_last_month: dec!(11.11),
        compared_with_last_year_same_month: dec!(25),
        accumulated_compared_with_last_year: dec!(25),
        date: 202001,
    }
}

/// 寫入當月營收並帶上該月日線的最低、平均、最高價；沒有日線的股票價格欄維持 0；
/// 寫完後最新營收日期表指向這個月份。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn save_revenues_writes_prices_and_rebuilds_latest_dates() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 save_revenues_writes_prices_and_rebuilds_latest_dates：無資料庫連接");
        return;
    }
    cleanup().await;
    let pool = database::get_connection();
    for symbol in [SYMBOL_A, SYMBOL_B] {
        sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '營收測試', $1, 2, false)"#,
            )
            .bind(symbol)
            .execute(pool)
            .await
            .expect("種入股票主檔");
    }
    sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice")
               VALUES ('2020-01-02', $1, 2020, 1, 2, 100, 104, 98, 102),
                      ('2020-01-03', $1, 2020, 1, 3, 102, 110, 101, 108)"#,
        )
        .bind(SYMBOL_A)
        .execute(pool)
        .await
        .expect("種入日線");

    save_revenues(
        vec![command(SYMBOL_A, dec!(1000)), command(SYMBOL_B, dec!(500))],
        2020,
        1,
    )
    .await
    .expect("save_revenues");

    let rows: Vec<(String, Decimal, Decimal, Decimal, Decimal)> = sqlx::query_as(
            r#"SELECT stock_symbol, "Monthly", lowest_price, avg_price, highest_price FROM "Revenue" WHERE stock_symbol IN ($1, $2) AND "Date" = 202001 ORDER BY stock_symbol"#,
        )
        .bind(SYMBOL_A)
        .bind(SYMBOL_B)
        .fetch_all(pool)
        .await
        .expect("讀回營收");
    assert_eq!(
        rows,
        vec![
            (
                SYMBOL_A.to_owned(),
                dec!(1000),
                dec!(98),
                dec!(105),
                dec!(110)
            ),
            (
                SYMBOL_B.to_owned(),
                dec!(500),
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO
            ),
        ]
    );
    let latest: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM revenue_last_date l JOIN "Revenue" r ON r."Serial" = l.serial WHERE l.stock_symbol IN ($1, $2) AND r."Date" = 202001"#,
        )
        .bind(SYMBOL_A)
        .bind(SYMBOL_B)
        .fetch_one(pool)
        .await
        .expect("讀回最新營收日期");
    assert_eq!(latest, 2);

    cleanup().await;
}

/// 以本機 mock server 取代公開資訊觀測站（G15）：抓 1991-01 月營收，逐檔寫入並重建最新營收日期。
mod mock_sources {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;
    use crate::domain::registry::entity::Stock;

    /// 假代號（月營收頁只收全數字代號）：C 在共享快取有名稱，D 沒有。
    const SYMBOL_C: &str = "79941";
    const SYMBOL_D: &str = "79942";

    /// 上市第 0 頁：兩檔月營收（欄位順序同公開資訊觀測站 t21sc03）。
    fn revenue_page() -> Vec<u8> {
        let html = format!(
            r#"<html><body><table>
<tr><th>公司代號</th><th>公司名稱</th><th>當月營收</th><th>上月營收</th><th>去年當月營收</th>
    <th>當月累計</th><th>去年累計</th><th>上月增減%</th><th>去年增減%</th><th>前期增減%</th></tr>
<tr><td>{SYMBOL_C}</td><td>營收測試甲</td><td>1,000</td><td>900</td><td>800</td>
    <td>1,000</td><td>800</td><td>11.11</td><td>25.00</td><td>25.00</td></tr>
<tr><td>{SYMBOL_D}</td><td>營收測試乙</td><td>500</td><td>450</td><td>400</td>
    <td>500</td><td>400</td><td>11.11</td><td>25.00</td><td>25.00</td></tr>
</table></body></html>"#
        );
        let (big5, _, _) = encoding_rs::BIG5.encode(&html);
        big5.into_owned()
    }

    async fn cleanup_mock() {
        let pool = database::get_connection();
        for symbol in [SYMBOL_C, SYMBOL_D] {
            for sql in [
                "DELETE FROM revenue_last_date WHERE stock_symbol = $1",
                r#"DELETE FROM "Revenue" WHERE stock_symbol = $1"#,
                "DELETE FROM stocks WHERE stock_symbol = $1",
            ] {
                sqlx::query(sql)
                    .bind(symbol)
                    .execute(pool)
                    .await
                    .unwrap_or_else(|why| panic!("{sql}: {why}"));
            }
        }
        if let Ok(mut cache) = SHARE.stocks.write() {
            cache.remove(SYMBOL_C);
        }
    }

    /// 1991-02-15 執行 → 抓 1991-01（民國 80 年 1 月）上市、上櫃各兩頁；只有上市第 0 頁有資料，
    /// 其餘 404 略過。兩檔寫入 `Revenue` 並指到最新營收日期。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn execute_on_fetches_previous_month_and_saves_revenues() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 execute_on_fetches_previous_month_and_saves_revenues：無資料庫連接");
            return;
        }
        cleanup_mock().await;
        let pool = database::get_connection();
        for symbol in [SYMBOL_C, SYMBOL_D] {
            sqlx::query(
                    r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '營收測試', $1, 2, false)"#,
                )
                .bind(symbol)
                .execute(pool)
                .await
                .expect("種入股票主檔");
        }
        if let Ok(mut cache) = SHARE.stocks.write() {
            cache.insert(
                SYMBOL_C.to_owned(),
                Stock::register(SYMBOL_C.to_owned(), "營收測試甲".to_owned(), 2, 1),
            );
        }

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nas/t21/sii/t21sc03_80_1_0.html"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(revenue_page(), "text/html"))
            .expect(1)
            .mount(&server)
            .await;
        let uri = server.uri();
        let today = NaiveDate::from_ymd_opt(1991, 2, 15).expect("有效日期");

        let result = with_origins(&[("https://mopsov.twse.com.tw", &uri)], execute_on(today)).await;
        let rows: Vec<(String, Decimal)> = sqlx::query_as(
                r#"SELECT stock_symbol, "Monthly" FROM "Revenue" WHERE stock_symbol IN ($1, $2) AND "Date" = 199101 ORDER BY stock_symbol"#,
            )
            .bind(SYMBOL_C)
            .bind(SYMBOL_D)
            .fetch_all(pool)
            .await
            .expect("讀回營收");
        let latest: i64 = sqlx::query_scalar(
                r#"SELECT count(*) FROM revenue_last_date l JOIN "Revenue" r ON r."Serial" = l.serial WHERE l.stock_symbol IN ($1, $2) AND r."Date" = 199101"#,
            )
            .bind(SYMBOL_C)
            .bind(SYMBOL_D)
            .fetch_one(pool)
            .await
            .expect("讀回最新營收日期");
        let requests = server
            .received_requests()
            .await
            .map_or(0, |requests| requests.len());
        cleanup_mock().await;

        result.expect("月營收排程完成");
        assert_eq!(
            rows,
            vec![
                (SYMBOL_C.to_owned(), dec!(1000)),
                (SYMBOL_D.to_owned(), dec!(500))
            ],
            "兩檔 1991-01 月營收都寫入"
        );
        assert_eq!(latest, 2, "最新營收日期指到 1991-01");
        assert_eq!(requests, 4, "上市、上櫃各兩頁，404 的頁面略過");
    }
}

#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
    tokio::time::sleep(Duration::from_secs(1)).await;
}
#[tokio::test]
#[ignore]
async fn test_process_revenues() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 test_process_revenues");

    let naive_datetime = NaiveDate::from_ymd_opt(2025, 4, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let month_timezone = timezone.from_local_datetime(&naive_datetime).unwrap();

    match process_revenues(month_timezone).await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to test_process_revenues because {:?}", why);
        }
    }

    tracing::debug!("結束 test_process_revenues");
    tokio::time::sleep(Duration::from_secs(1)).await;
}
