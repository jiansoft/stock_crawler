use chrono::{Datelike, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::core::declare::StockExchange;
use crate::infra::cache::SHARE;
use crate::infra::crawler::twse;

use super::super::FromWithExchange;
use super::*;

#[tokio::test]
#[ignore]
async fn test_fetch_moving_average() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_moving_average");
    let date = NaiveDate::from_ymd_opt(2023, 8, 1);
    let mut dq = DailyQuote::new("2330".to_string());
    dq.date = date.unwrap();
    match dq.fill_moving_average().await {
        Ok(_) => {
            dbg!(&dq);
            tracing::debug!("fetch_moving_average: {:#?}", dq);
        }
        Err(why) => {
            tracing::debug!("Failed to fetch_moving_average because {:?}", why);
        }
    }

    tracing::debug!("結束 fetch_moving_average");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 upsert");

    let data = vec![
        "79979".to_string(),
        "台泥".to_string(),
        "28,131,977".to_string(),
        "12,278".to_string(),
        "1,070,452,844".to_string(),
        "37.65".to_string(),
        "38.30".to_string(),
        "37.65".to_string(),
        "37.95".to_string(),
        "<p style= color:red>+</p>".to_string(),
        "0.40".to_string(),
        "37.95".to_string(),
        "139".to_string(),
        "38.00".to_string(),
        "309".to_string(),
        "51.28".to_string(),
    ];

    let mut e = DailyQuote::from_with_exchange(StockExchange::TWSE, &data);
    e.date = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    e.year = e.date.year();
    e.month = e.date.month() as i32;
    e.day = e.date.day() as i32;
    e.record_time = Local::now();
    e.create_time = Local::now();

    match e.upsert().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to upsert because:{:?}", why);
        }
    }

    let otc = vec![
        "79979".to_string(),
        "茂生農經".to_string(),
        "46.55".to_string(),
        "+0.30".to_string(),
        "46.25".to_string(),
        "46.80".to_string(),
        "46.25".to_string(),
        "78,000".to_string(),
        "3,632,550".to_string(),
        "63".to_string(),
        "46.30".to_string(),
        "2".to_string(),
        "46.60".to_string(),
        "2".to_string(),
        "38,598,194".to_string(),
        "51.20".to_string(),
        "41.90".to_string(),
    ];

    let mut e = DailyQuote::from_with_exchange(StockExchange::TPEx, &otc);
    e.date = NaiveDate::from_ymd_opt(2000, 1, 2).unwrap();
    e.year = e.date.year();
    e.month = e.date.month() as i32;
    e.day = e.date.day() as i32;
    e.record_time = Local::now();
    e.create_time = Local::now();

    match e.upsert().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to upsert because:{:?}", why);
        }
    }

    // 測試結束後移除假代號資料列，避免測試資料殘留在資料庫
    // （曾在正式庫發現本測試留下的 '79979' 收盤紀錄）。
    if let Err(why) = sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE "stock_symbol" = $1"#)
        .bind("79979")
        .execute(database::get_connection())
        .await
    {
        tracing::debug!("Failed to cleanup test rows because:{:?}", why);
    }

    tracing::debug!("結束 upsert");
}

/// 空批次沒有任何可更新的目標，必須明確報錯而不是送出一句無效 SQL。
#[tokio::test]
async fn batch_update_moving_average_rejects_an_empty_batch() {
    let err = DailyQuote::batch_update_moving_average(&[])
        .await
        .expect_err("空批次應回傳錯誤");
    assert!(
        err.to_string().contains("empty"),
        "錯誤訊息應說明原因：{err}"
    );
}

/// 空批次在碰資料庫之前就短路回 0，回補流程可以放心傳空清單。
#[tokio::test]
async fn insert_missing_batch_short_circuits_on_an_empty_slice() {
    assert_eq!(
        DailyQuote::insert_missing_batch(&[])
            .await
            .expect("空批次應成功"),
        0
    );
}

/// 建立一筆最小可寫入的測試報價（固定歷史日期 + 假代號）。
fn make_quote(symbol: &str, date: NaiveDate, closing_price: rust_decimal::Decimal) -> DailyQuote {
    let mut quote = DailyQuote::new(symbol.to_string());
    quote.date = date;
    quote.year = date.year();
    quote.month = date.month() as i32;
    quote.day = date.day() as i32;
    quote.closing_price = closing_price;
    quote.opening_price = closing_price;
    quote.highest_price = closing_price;
    quote.lowest_price = closing_price;
    quote
}

/// 回補歷史缺口時只補空位：新日期寫入，既有日期既不重複也不被覆寫。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn insert_missing_batch_fills_gaps_without_overwriting() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 insert_missing_batch_fills_gaps_without_overwriting：無資料庫連接");
        return;
    }

    // 固定歷史日期 + 假代號，避免碰到任何真實資料。
    const SYMBOL: &str = "79979";
    let day1 = NaiveDate::from_ymd_opt(1970, 1, 1).expect("測試日期應合法");
    let day2 = NaiveDate::from_ymd_opt(1970, 1, 2).expect("測試日期應合法");
    let cleanup = || async {
        let _ = sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE "stock_symbol" = $1"#)
            .bind(SYMBOL)
            .execute(database::get_connection())
            .await;
    };
    cleanup().await;

    // 首次寫入兩天，兩筆都是新資料。
    let inserted = DailyQuote::insert_missing_batch(&[
        make_quote(SYMBOL, day1, rust_decimal::Decimal::from(10)),
        make_quote(SYMBOL, day2, rust_decimal::Decimal::from(11)),
    ])
    .await
    .expect("首次寫入應成功");
    assert_eq!(inserted, 2);

    // 重跑同一批並多帶一天：只有新的那天會被寫入（ON CONFLICT DO NOTHING）。
    let day3 = NaiveDate::from_ymd_opt(1970, 1, 3).expect("測試日期應合法");
    let inserted = DailyQuote::insert_missing_batch(&[
        make_quote(SYMBOL, day1, rust_decimal::Decimal::from(99)),
        make_quote(SYMBOL, day2, rust_decimal::Decimal::from(99)),
        make_quote(SYMBOL, day3, rust_decimal::Decimal::from(12)),
    ])
    .await
    .expect("重跑應成功");
    assert_eq!(inserted, 1);

    // 既有列的價格不得被重跑的值覆寫——補洞不該動到已有資料。
    let closing: rust_decimal::Decimal = sqlx::query_scalar(
        r#"SELECT "ClosingPrice" FROM "DailyQuotes" WHERE "stock_symbol" = $1 AND "Date" = $2"#,
    )
    .bind(SYMBOL)
    .bind(day1)
    .fetch_one(database::get_connection())
    .await
    .expect("應查得到既有資料");
    assert_eq!(closing, rust_decimal::Decimal::from(10));

    cleanup().await;
}

/// COPY 寫入驗證：以 TWSE 每日收盤行情 fixture（100 檔）走 ACL 轉成 [`DailyQuote`]，
/// 在交易內 COPY、驗證筆數後回滾，不留下任何資料列。
///
/// 舊版改抓 TWSE 即時資料、COPY 後不清理，且標成 `#[ignore]` 平常不跑；2026-06-30 對正式庫
/// 執行後留下 1,204 筆日期為 1970-01-01 的重複報價（2026-10-05 才清除）。現在不連外部網站，
/// 隨整合測試一起執行。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_copy_in_raw_rolls_back() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_copy_in_raw_rolls_back：無資料庫連接");
        return;
    }

    let response: twse::quote::ListedResponse = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/twse_quote.json"
    ))
    .expect("TWSE fixture should parse");
    let table = &response.tables[0];
    let field_map: std::collections::HashMap<&str, usize> = table
        .fields
        .as_ref()
        .expect("fixture fields")
        .iter()
        .enumerate()
        .map(|(index, field)| (field.as_str(), index))
        .collect();
    // 用不會與真實資料衝突的日期，避免撞到 (stock_symbol, Date) 唯一索引；交易結束即回滾。
    let date = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let quotes: Vec<DailyQuote> = table
        .data
        .as_ref()
        .expect("fixture rows")
        .iter()
        .map(|row| {
            let dto =
                crate::infra::crawler::share::DailyQuoteDto::from_with_map(row, &field_map, date)
                    .expect("fixture row should parse");
            let cmd = crate::app::backfill::acl::QuoteAclMapper::from_dto(&dto);
            DailyQuote::from(crate::app::backfill::acl::QuoteAclMapper::from_command(
                &cmd,
            ))
        })
        .collect();
    assert!(!quotes.is_empty());

    let count_on_date = r#"SELECT COUNT(*) FROM "DailyQuotes" WHERE "Date" = $1"#;
    let mut tx = database::get_connection()
        .begin()
        .await
        .expect("begin transaction");
    let copied = DailyQuote::copy_in_raw_on(&mut tx, &quotes)
        .await
        .expect("copy_in_raw_on should succeed");
    assert_eq!(copied as usize, quotes.len());
    let inside: i64 = sqlx::query_scalar(count_on_date)
        .bind(date)
        .fetch_one(&mut *tx)
        .await
        .expect("count inside transaction");
    assert_eq!(
        inside as usize,
        quotes.len(),
        "交易內應看得到剛 COPY 的資料"
    );
    tx.rollback().await.expect("rollback");

    let after: i64 = sqlx::query_scalar(count_on_date)
        .bind(date)
        .fetch_one(database::get_connection())
        .await
        .expect("count after rollback");
    assert_eq!(after, 0, "回滾後不可留下任何資料列");
}

/// 視窗函數重算的結果必須與逐日的 `fill_moving_average` 完全一致。
///
/// 79979 是連續 300 個平日（走 240 筆分支）；79978 每 3 個平日才一筆、跨兩年多
/// （400 天內不滿 240 筆，走日曆日分支）。收盤價刻意帶小數與重複值，涵蓋同價取日期。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn recalculate_moving_averages_matches_fill_moving_average() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 recalculate_moving_averages_matches_fill_moving_average：無資料庫連接");
        return;
    }

    let symbols = vec!["79979".to_string(), "79978".to_string()];
    let cleanup = || async {
        sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#)
            .bind(&symbols)
            .execute(database::get_connection())
            .await
            .expect("清除測試資料列");
    };
    cleanup().await;

    let insert = r#"
INSERT INTO "DailyQuotes" (stock_symbol, "Date", "ClosingPrice", "HighestPrice", "LowestPrice", year, month, day)
SELECT $1, d, c, c + (i % 4) * 0.5, c - (i % 3) * 0.25, date_part('year', d), date_part('month', d), date_part('day', d)
FROM (
    SELECT row_number() OVER (ORDER BY d) AS i, d::date AS d
    FROM generate_series(DATE '2001-01-01', DATE '2004-12-31', INTERVAL '1 day') AS d
    WHERE extract(isodow FROM d) < 6
) AS days,
LATERAL (SELECT 10 + (i % 17) + (i % 3) * 0.25 AS c) AS price
WHERE i % $2 = 0 AND i <= $3
"#;
    for (symbol, step, last) in [("79979", 1_i64, 300_i64), ("79978", 3, 900)] {
        sqlx::query(insert)
            .bind(symbol)
            .bind(step)
            .bind(last)
            .execute(database::get_connection())
            .await
            .expect("寫入測試資料列");
    }

    let from = NaiveDate::from_ymd_opt(2001, 1, 1).expect("日期應合法");
    let updated = DailyQuote::recalculate_moving_averages(&symbols, from)
        .await
        .expect("重算應成功");
    assert_eq!(updated, 600, "兩檔各 300 列都從全 0 變成有值");

    let again = DailyQuote::recalculate_moving_averages(&symbols, from)
        .await
        .expect("重算應成功");
    assert_eq!(again, 0, "重跑時數值沒變，不該再寫入");

    let dates: Vec<NaiveDate> = sqlx::query_scalar(
        r#"SELECT DISTINCT "Date" FROM "DailyQuotes" WHERE stock_symbol = ANY($1) ORDER BY 1"#,
    )
    .bind(&symbols)
    .fetch_all(database::get_connection())
    .await
    .expect("讀回測試日期");
    let mut rows = Vec::new();
    for date in dates {
        rows.extend(
            super::super::fetch_daily_quotes_by_date(date)
                .await
                .expect("讀回測試資料列")
                .into_iter()
                .filter(|row| symbols.contains(&row.stock_symbol)),
        );
    }
    assert_eq!(rows.len(), 600);

    let mut mismatches = Vec::new();
    for row in &rows {
        let mut expected = row.clone();
        expected
            .fill_moving_average()
            .await
            .expect("逐日計算應成功");
        let actual = [
            row.moving_average_5,
            row.moving_average_10,
            row.moving_average_20,
            row.moving_average_60,
            row.moving_average_120,
            row.moving_average_240,
            row.maximum_price_in_year,
            row.minimum_price_in_year,
            row.average_price_in_year,
        ];
        let wanted = [
            expected.moving_average_5,
            expected.moving_average_10,
            expected.moving_average_20,
            expected.moving_average_60,
            expected.moving_average_120,
            expected.moving_average_240,
            expected.maximum_price_in_year,
            expected.minimum_price_in_year,
            expected.average_price_in_year,
        ];
        // 同價時逐日算法取哪一天沒有定義，比對「該日的價格等於極值」即可。
        let price_on = |date: NaiveDate, pick: fn(&DailyQuote) -> Decimal| {
            rows.iter()
                .find(|r| r.stock_symbol == row.stock_symbol && r.date == date)
                .map(|r| pick(r).round_dp(2))
        };
        let max_on_ok = price_on(row.maximum_price_in_year_date_on, |r| r.highest_price)
            == Some(row.maximum_price_in_year);
        let min_on_ok = price_on(row.minimum_price_in_year_date_on, |r| r.lowest_price)
            == Some(row.minimum_price_in_year);
        if actual != wanted || !max_on_ok || !min_on_ok {
            mismatches.push((row.stock_symbol.clone(), row.date));
        }
    }

    cleanup().await;
    assert!(mismatches.is_empty(), "與逐日算法不一致：{mismatches:?}");
}
