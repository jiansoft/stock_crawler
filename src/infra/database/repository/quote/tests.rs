use super::*;
use crate::domain::quote::entity::LastDailyQuote;
use chrono::Datelike;
use rust_decimal_macros::dec;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_cache_aside_flow() {
    // 載入環境變數設定
    dotenvy::dotenv().ok();

    if database::ping().await.is_err() {
        println!("跳過 test_cache_aside_flow：無資料庫連接");
        return;
    }

    if CLIENT.ping().await.is_err() {
        println!("跳過 test_cache_aside_flow：無 Redis 連接");
        return;
    }

    // 建立測試專用的倉儲對象
    let repo = PgQuoteRepository::new();
    let test_symbol = "TEST_9999";

    // 先取得快取的 key
    let cache_key = repo.get_cache_key(test_symbol);
    // 清除先前殘留的 Redis 快取以確保測試獨立性
    let _ = CLIENT.delete(&cache_key).await;

    // 建立測試用的最新收盤價領域對象
    let test_quote = LastDailyQuote {
        date: NaiveDate::from_ymd_opt(2026, 6, 8).unwrap(),
        stock_symbol: test_symbol.to_string(),
        closing_price: dec!(99.9),
    };

    // 1. 執行批次寫入，內部會同時寫入資料庫與 Redis 快取
    repo.save_last_quotes_batch(std::slice::from_ref(&test_quote))
        .await
        .unwrap();

    // 2. 第一次讀取，此時預期可以直接從快取中命中（因為 batch 寫入時有回寫）
    let fetched_first = repo.fetch_last_quote(test_symbol).await.unwrap();
    assert!(fetched_first.is_some());
    let fetched_first = fetched_first.unwrap();
    assert_eq!(fetched_first.stock_symbol, test_symbol);
    assert_eq!(fetched_first.closing_price, dec!(99.9));

    // 3. 再次清除快取以模擬 Cache Miss 的情境
    let _ = CLIENT.delete(&cache_key).await;

    // 4. 第二次讀取，因快取已被清除，會觸發 Cache Miss 降級並從 PostgreSQL 重新查詢，最後會回寫快取
    let fetched_miss = repo.fetch_last_quote(test_symbol).await.unwrap();
    assert!(fetched_miss.is_some());
    assert_eq!(fetched_miss.unwrap().closing_price, dec!(99.9));

    // 5. 驗證此時 Redis 快取是否已正確被自動回寫
    let redis_val = CLIENT.get_string(&cache_key).await.unwrap();
    let redis_quote: LastDailyQuote = serde_json::from_str(&redis_val).unwrap();
    assert_eq!(redis_quote.stock_symbol, test_symbol);
    assert_eq!(redis_quote.closing_price, dec!(99.9));

    // 6. 清理測試資料（同時清除 Redis 快取與資料庫內的測試列）
    let _ = CLIENT.delete(&cache_key).await;
    let _ = sqlx::query("DELETE FROM last_daily_quotes WHERE stock_symbol = $1")
        .bind(test_symbol)
        .execute(database::get_connection())
        .await;
}

/// 空清單在轉換與碰資料庫之前就短路回 0。
///
/// 回補流程對「這批沒有缺口」的情況會傳空清單進來，屬於正常路徑。
#[tokio::test]
async fn insert_missing_daily_quotes_short_circuits_on_an_empty_slice() {
    let repository = PgQuoteRepository::new();

    assert_eq!(
        repository
            .insert_missing_daily_quotes(&[])
            .await
            .expect("空清單應成功"),
        0
    );
}

/// 經由倉儲介面補寫缺口：領域實體正確轉成 Table 模型，且只補空位。
///
/// 與 table 層的同名測試分工不同：這裡驗證的是 domain → table 的轉換有沒有
/// 把日期與代號帶到位，寫入語意本身由 table 層那支負責。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn insert_missing_daily_quotes_maps_and_fills_gaps() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 insert_missing_daily_quotes_maps_and_fills_gaps：無資料庫連接");
        return;
    }

    // 固定歷史日期 + 假代號，避免碰到任何真實資料。
    const SYMBOL: &str = "79978";
    let day1 = NaiveDate::from_ymd_opt(1970, 1, 1).expect("測試日期應合法");
    let day2 = NaiveDate::from_ymd_opt(1970, 1, 2).expect("測試日期應合法");
    let cleanup = || async {
        let _ = sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE "stock_symbol" = $1"#)
            .bind(SYMBOL)
            .execute(database::get_connection())
            .await;
    };
    cleanup().await;

    let quote = |date: NaiveDate| DomainDailyQuote {
        stock_symbol: SYMBOL.to_string(),
        date,
        year: date.year(),
        month: date.month() as i32,
        day: date.day() as i32,
        closing_price: dec!(10),
        ..Default::default()
    };

    let repository = PgQuoteRepository::new();
    assert_eq!(
        repository
            .insert_missing_daily_quotes(&[quote(day1), quote(day2)])
            .await
            .expect("首次寫入應成功"),
        2
    );
    // 重跑不得產生重複資料列。
    assert_eq!(
        repository
            .insert_missing_daily_quotes(&[quote(day1), quote(day2)])
            .await
            .expect("重跑應成功"),
        0
    );

    // 轉換有把代號與日期帶到位，資料才查得回來。
    let stored: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM "DailyQuotes" WHERE "stock_symbol" = $1 AND "Date" IN ($2, $3)"#,
    )
    .bind(SYMBOL)
    .bind(day1)
    .bind(day2)
    .fetch_one(database::get_connection())
    .await
    .expect("應查得到寫入的資料");
    assert_eq!(stored, 2);

    cleanup().await;
}

/// Table／Domain 之間的轉換欄位一一對應，來回轉換不遺失資料。
#[test]
fn conversions_round_trip_without_losing_fields() {
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
    let quote = DomainDailyQuote {
        serial: 7,
        stock_symbol: "2330".to_string(),
        date,
        opening_price: dec!(1440),
        highest_price: dec!(1455),
        lowest_price: dec!(1435),
        closing_price: dec!(1450),
        change: dec!(10),
        change_range: dec!(0.69),
        trading_volume: dec!(30000),
        price_earning_ratio: dec!(29.44),
        price_to_book_ratio: dec!(10.24),
        moving_average_5: dec!(1430),
        moving_average_240: dec!(1100),
        maximum_price_in_year: dec!(1500),
        minimum_price_in_year_date_on: date,
        year: 2026,
        month: 10,
        day: 12,
        ..Default::default()
    };
    assert_eq!(
        DomainDailyQuote::from(TableDailyQuote::from(quote.clone())),
        quote
    );

    let last = DomainLastDailyQuote {
        date,
        closing_price: dec!(1450),
        stock_symbol: "2330".to_string(),
    };
    assert_eq!(
        DomainLastDailyQuote::from(TableLastDailyQuotes::from(last.clone())),
        last
    );

    let record = DomainQuoteHistoryRecord {
        maximum_price: dec!(1500),
        minimum_price: dec!(10),
        maximum_price_to_book_ratio: dec!(12.5),
        minimum_price_to_book_ratio: dec!(0.8),
        maximum_price_date_on: date,
        ..DomainQuoteHistoryRecord::new("2330".to_string())
    };
    assert_eq!(
        DomainQuoteHistoryRecord::from(TableQuoteHistoryRecord::from(record.clone())),
        record
    );
}

/// 經由倉儲介面走一輪日報價讀寫：單筆與批次寫入、整日替換、月統計、均線、
/// 市場成交檔數、補缺、歷史極值、最新收盤、全市場統計，最後刪除整日資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn repository_round_trips_daily_quote_data() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 repository_round_trips_daily_quote_data：無資料庫連接");
        return;
    }

    // 固定歷史日期 + 假代號；day3 只用來驗證補缺。
    const SYMBOL: &str = "79978QR";
    let day = |d| NaiveDate::from_ymd_opt(1989, 8, d).expect("測試日期應合法");
    let (day1, day2, day3) = (day(1), day(2), day(3));
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
            "DELETE FROM last_daily_quotes WHERE stock_symbol = $1",
            "DELETE FROM quote_history_record WHERE security_code = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
        }
        let _ = sqlx::query("DELETE FROM daily_stock_price_stats WHERE date = $1")
            .bind(day1)
            .execute(pool)
            .await;
        let _ = CLIENT.delete(&format!("LastDailyQuote:{SYMBOL}")).await;
    };
    cleanup().await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ($1, $1, $1, 2, 1)"#,
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入 stocks");

    let quote = |date: NaiveDate, close: Decimal| DomainDailyQuote {
        stock_symbol: SYMBOL.to_string(),
        date,
        year: date.year(),
        month: date.month() as i32,
        day: date.day() as i32,
        opening_price: close,
        highest_price: close + dec!(1),
        lowest_price: close - dec!(1),
        closing_price: close,
        trading_volume: dec!(1000),
        ..Default::default()
    };
    let repo = PgQuoteRepository::new();

    repo.save_daily_quote(&quote(day1, dec!(10)))
        .await
        .expect("單筆寫入");
    repo.batch_save_daily_quotes(&[quote(day2, dec!(12))])
        .await
        .expect("批次寫入");
    let fetched = repo.fetch_quotes_by_date(day2).await.expect("讀取整日");
    assert!(
        fetched
            .iter()
            .any(|q| q.stock_symbol == SYMBOL && q.closing_price == dec!(12))
    );
    assert_eq!(
        repo.replace_quotes_by_date(day2, &[quote(day2, dec!(14))])
            .await
            .expect("整日替換"),
        1
    );

    // 月統計：最低 9（day1 低點）、平均收盤 12、最高 15（day2 高點）。
    assert_eq!(
        repo.fetch_monthly_stock_price_summary(SYMBOL, 1989, 8)
            .await
            .expect("月統計"),
        Some((dec!(9), dec!(12), dec!(15)))
    );
    assert!(
        repo.fetch_monthly_stock_price_summary(SYMBOL, 1989, 9)
            .await
            .expect("無資料月份")
            .is_none()
    );

    let mut latest = quote(day2, dec!(14));
    repo.fill_moving_average(&mut latest)
        .await
        .expect("計算均線");
    assert_eq!(latest.stock_symbol, SYMBOL);
    repo.batch_update_moving_average(&[latest])
        .await
        .expect("批次更新均線");
    repo.recalculate_moving_averages(&[SYMBOL.to_string()], day1)
        .await
        .expect("重算均線");

    assert!(
        repo.fetch_symbols_quoted_since(day1)
            .await
            .expect("期間有報價的代號")
            .iter()
            .any(|symbol| symbol == SYMBOL)
    );
    assert!(
        repo.fetch_market_traded_counts(day1, day2)
            .await
            .expect("市場成交檔數")
            .iter()
            .any(|count| count.date == day1 && count.market_id == 2 && count.traded >= 1)
    );

    // day3 沒有報價：補缺沿用 day2 收盤、成交量為 0。
    assert!(
        repo.makeup_for_the_lack_daily_quotes(day3)
            .await
            .expect("補缺")
            >= 1
    );
    let (close, volume): (Decimal, Decimal) = sqlx::query_as(
        r#"SELECT "ClosingPrice", "TradingVolume" FROM "DailyQuotes" WHERE stock_symbol = $1 AND "Date" = $2"#,
    )
    .bind(SYMBOL)
    .bind(day3)
    .fetch_one(pool)
    .await
    .expect("補缺列");
    assert_eq!((close, volume), (dec!(14), dec!(0)));

    // 歷史極值：最高 15（day2）、最低 9（day1）；再以倉儲寫入覆蓋淨值比極值。
    assert!(
        repo.rebuild_quote_history_price_extremes(&[SYMBOL.to_string()])
            .await
            .expect("重建歷史極值")
            >= 1
    );
    let mut record = repo
        .fetch_quote_history_records()
        .await
        .expect("讀取歷史極值")
        .into_iter()
        .find(|record| record.security_code == SYMBOL)
        .expect("應有測試代號的極值");
    assert_eq!(
        (record.maximum_price, record.maximum_price_date_on),
        (dec!(15), day2)
    );
    assert_eq!(
        (record.minimum_price, record.minimum_price_date_on),
        (dec!(9), day1)
    );
    record.maximum_price_to_book_ratio = dec!(3.5);
    repo.save_quote_history_record(&record)
        .await
        .expect("寫入歷史極值");
    let saved: Decimal = sqlx::query_scalar(
        r#"SELECT "maximum_price-to-book_ratio" FROM quote_history_record WHERE security_code = $1"#,
    )
    .bind(SYMBOL)
    .fetch_one(pool)
    .await
    .expect("讀回淨值比極值");
    assert_eq!(saved, dec!(3.5));

    // 重建只取近 30 天的報價，歷史測試日期不會入表；改以批次寫入驗證讀取。
    repo.rebuild_last_daily_quotes()
        .await
        .expect("重建最新收盤");
    repo.save_last_quotes_batch(&[])
        .await
        .expect("空清單直接成功");
    repo.save_last_quotes_batch(&[DomainLastDailyQuote {
        date: day3,
        closing_price: dec!(14),
        stock_symbol: SYMBOL.to_string(),
    }])
    .await
    .expect("寫入最新收盤");
    let last = repo
        .fetch_last_daily_quotes()
        .await
        .expect("讀取最新收盤")
        .into_iter()
        .find(|quote| quote.stock_symbol == SYMBOL)
        .expect("應有測試代號的最新收盤");
    assert_eq!((last.date, last.closing_price), (day3, dec!(14)));

    repo.save_stock_price_stats(day1).await.expect("全市場統計");

    repo.delete_quotes_by_date(day3).await.expect("刪除整日");
    assert!(
        repo.fetch_quotes_by_date(day3)
            .await
            .expect("讀取已刪除的日期")
            .iter()
            .all(|q| q.stock_symbol != SYMBOL)
    );

    cleanup().await;
}
