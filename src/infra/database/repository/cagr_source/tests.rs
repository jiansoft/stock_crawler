use super::*;
use rust_decimal_macros::dec;

/// 測試專用的假代號；真實市場不存在此代號。
const FAKE_A: &str = "79979S1";
/// 已下市的假代號，用於驗證母體排除規則。
const FAKE_B: &str = "79979S2";
/// 驗證上市初期與公司行動生效日落在兩筆報價之間的假代號。
const FAKE_C: &str = "79979S3";

/// 測試資料一律落在 1990 年初 —— 遠早於 `DailyQuotes` 實際涵蓋的 2012 年，
/// 既不會與正式資料互相干擾，也仍在哨兵值 `1990-01-01` 之後。
fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1990, 1, d).expect("測試日期應合法")
}

async fn cleanup() {
    let symbols = vec![FAKE_A.to_string(), FAKE_B.to_string(), FAKE_C.to_string()];
    let _ = sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#)
        .bind(&symbols)
        .execute(database::get_connection())
        .await;
    let _ = sqlx::query("DELETE FROM dividend WHERE security_code = ANY($1)")
        .bind(&symbols)
        .execute(database::get_connection())
        .await;
    let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
        .bind(&symbols)
        .execute(database::get_connection())
        .await;
    let _ = sqlx::query("DELETE FROM corporate_action WHERE stock_symbol = ANY($1)")
        .bind(&symbols)
        .execute(database::get_connection())
        .await;
}

/// 在 1989 年底補 5 筆前置報價，讓 1990 年初的跳動不落在上市初期。
async fn insert_listing_history(symbol: &str) {
    for d in 18..=22 {
        insert_quote(
            symbol,
            NaiveDate::from_ymd_opt(1989, 12, d).unwrap(),
            dec!(100),
        )
        .await;
    }
}

async fn insert_corporate_action(symbol: &str, date: NaiveDate, ratio: Decimal) {
    sqlx::query(
        r#"INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio, note)
           VALUES ($1, $2, 'split', $3, '測試用')
           ON CONFLICT (stock_symbol, effective_date) DO UPDATE SET share_ratio = excluded.share_ratio"#,
    )
    .bind(symbol)
    .bind(date)
    .bind(ratio)
    .execute(database::get_connection())
    .await
    .expect("插入公司行動");
}

async fn insert_quote(symbol: &str, date: NaiveDate, close: Decimal) {
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
           VALUES ($1, $2, $3, 1000)"#,
    )
    .bind(date)
    .bind(symbol)
    .bind(close)
    .execute(database::get_connection())
    .await
    .expect("插入報價");
}

async fn insert_stock(symbol: &str, suspend: bool) {
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, "SuspendListing")
           VALUES ($1, $1, $1, $2)
           ON CONFLICT (stock_symbol) DO UPDATE SET "SuspendListing" = excluded."SuspendListing""#,
    )
    .bind(symbol)
    .bind(suspend)
    .execute(database::get_connection())
    .await
    .expect("插入股票母檔");
}

#[allow(clippy::too_many_arguments)]
async fn insert_dividend(
    symbol: &str,
    year: i32,
    quarter: &str,
    cash: Decimal,
    stock: Decimal,
    date1: &str,
    date2: &str,
) {
    sqlx::query(
        r#"INSERT INTO dividend (security_code, year, quarter, cash_dividend, stock_dividend,
                                 "ex-dividend_date1", "ex-dividend_date2")
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (security_code, year, year_of_dividend, quarter) DO UPDATE
               SET cash_dividend = excluded.cash_dividend"#,
    )
    .bind(symbol)
    .bind(year)
    .bind(quarter)
    .bind(cash)
    .bind(stock)
    .bind(date1)
    .bind(date2)
    .execute(database::get_connection())
    .await
    .expect("插入股利");
}

/// 建立測試資料集。
///
/// `79979S1` 的價格序列刻意安排兩次大跳動：01-03 那次有對應的除息日
/// （不得標記為異常），01-05 那次沒有（必須標記）。
async fn seed() {
    cleanup().await;

    insert_stock(FAKE_A, false).await;
    insert_stock(FAKE_B, true).await;

    // 1970-01-01 是資料庫中實際存在的預設哨兵值，必須被所有查詢排除。
    insert_quote(
        FAKE_A,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
        dec!(1),
    )
    .await;
    insert_quote(FAKE_A, day(2), dec!(100)).await;
    insert_quote(FAKE_A, day(3), dec!(160)).await;
    // 收盤價為零者視同無報價。
    insert_quote(FAKE_A, day(4), Decimal::ZERO).await;
    insert_quote(FAKE_A, day(5), dec!(300)).await;
    insert_quote(FAKE_A, day(8), dec!(310)).await;
    insert_quote(FAKE_B, day(3), dec!(50)).await;

    // 年度彙總列與同年季度明細列並存：只能採計明細，否則股利算兩次。
    insert_dividend(FAKE_A, 1990, "", dec!(5), Decimal::ZERO, "1990-01-03", "-").await;
    insert_dividend(
        FAKE_A,
        1990,
        "Q1",
        dec!(2),
        Decimal::ZERO,
        "1990-01-03",
        "-",
    )
    .await;
    // 除息日為髒值、除權日合法：仍應取得事件，但現金除息日為 None。
    insert_dividend(FAKE_A, 1991, "", dec!(3), dec!(1), "1.39%", "1991-03-01").await;
    // 兩個日期都是髒值 —— 無法定位於時間軸，必須整筆略過。
    insert_dividend(FAKE_B, 1990, "", dec!(1), Decimal::ZERO, "尚未公布", "-").await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_trading_day_and_symbol_queries() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_trading_day_and_symbol_queries：無資料庫連接");
        return;
    }
    seed().await;
    let repo = PgCagrSourceRepository::new();

    // 對齊到不晚於指定日的交易日；1990-01-04 收盤價為零但仍是交易日。
    assert_eq!(
        repo.fetch_trading_day_on_or_before(day(7))
            .await
            .expect("fetch_trading_day_on_or_before"),
        Some(day(5))
    );
    assert_eq!(
        repo.fetch_trading_day_on_or_before(day(3))
            .await
            .expect("fetch_trading_day_on_or_before"),
        Some(day(3))
    );
    // 哨兵值 1970-01-01 不得被當成交易日。
    assert_eq!(
        repo.fetch_trading_day_on_or_before(NaiveDate::from_ymd_opt(1989, 12, 31).unwrap())
            .await
            .expect("fetch_trading_day_on_or_before"),
        None
    );

    let latest = repo
        .fetch_latest_trading_day()
        .await
        .expect("fetch_latest_trading_day");
    assert!(latest.is_some_and(|d| d >= day(8)));

    let symbols = repo
        .fetch_active_symbols()
        .await
        .expect("fetch_active_symbols");
    assert!(symbols.iter().any(|s| s == FAKE_A));
    assert!(
        !symbols.iter().any(|s| s == FAKE_B),
        "已下市股票不得進入計算母體"
    );

    let first_quotes = repo
        .fetch_first_quote_dates()
        .await
        .expect("fetch_first_quote_dates");
    let first = first_quotes
        .iter()
        .find(|(symbol, _)| symbol == FAKE_A)
        .map(|(_, date)| *date);
    assert_eq!(first, Some(day(2)), "哨兵值 1970-01-01 必須被排除");

    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_price_queries_skip_zero_and_respect_ranges() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_price_queries_skip_zero_and_respect_ranges：無資料庫連接");
        return;
    }
    seed().await;
    let repo = PgCagrSourceRepository::new();

    let prices = repo
        .fetch_closing_prices_on(day(3))
        .await
        .expect("fetch_closing_prices_on");
    assert_eq!(
        prices
            .iter()
            .find(|(symbol, _)| symbol == FAKE_A)
            .map(|(_, price)| *price),
        Some(dec!(160))
    );

    // 收盤價為零者不得出現。
    let zero_day = repo
        .fetch_closing_prices_on(day(4))
        .await
        .expect("fetch_closing_prices_on");
    assert!(!zero_day.iter().any(|(symbol, _)| symbol == FAKE_A));

    // 寬限查詢的區間是左開右閉：期初日當天的報價不算，往後第一筆才算。
    let grace = repo
        .fetch_first_quote_within(day(2), day(6))
        .await
        .expect("fetch_first_quote_within");
    let hit = grace.iter().find(|(symbol, _, _)| symbol == FAKE_A);
    assert_eq!(
        hit.map(|(_, date, price)| (*date, *price)),
        Some((day(3), dec!(160)))
    );

    // 批次查價：命中者回傳，查無者靜默略過（不得整批失敗）。
    let pairs = vec![
        (FAKE_A.to_string(), day(5)),
        (FAKE_A.to_string(), day(6)),
        (FAKE_A.to_string(), day(4)),
    ];
    let at = repo
        .fetch_closing_prices_at(&pairs)
        .await
        .expect("fetch_closing_prices_at");
    assert_eq!(at.len(), 1);
    assert_eq!(at[0], (FAKE_A.to_string(), day(5), dec!(300)));

    // 空輸入不應送出查詢。
    assert!(
        repo.fetch_closing_prices_at(&[])
            .await
            .expect("fetch_closing_prices_at empty")
            .is_empty()
    );

    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_dividend_events_deduplicate_and_tolerate_dirty_dates() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_dividend_events_deduplicate_and_tolerate_dirty_dates：無資料庫連接");
        return;
    }
    seed().await;
    let repo = PgCagrSourceRepository::new();

    let events = repo
        .fetch_dividend_events_since(day(1))
        .await
        .expect("fetch_dividend_events_since");
    let mine: Vec<&DividendEvent> = events
        .iter()
        .filter(|event| event.stock_symbol == FAKE_A || event.stock_symbol == FAKE_B)
        .collect();

    // 1990 年只能取到季度明細（現金 2），年度彙總列（現金 5）必須被去重掉，
    // 否則季配息股的股利會被計算兩次。
    let nineteen_ninety: Vec<&&DividendEvent> = mine
        .iter()
        .filter(|event| event.ex_dividend_date_cash == Some(day(3)))
        .collect();
    assert_eq!(nineteen_ninety.len(), 1, "年度彙總列與明細列不得同時採計");
    assert_eq!(nineteen_ninety[0].cash_dividend, dec!(2));

    // 除息日是 '1.39%' 這種髒值時只丟棄該日期，事件本身仍憑除權日成立。
    let dirty = mine
        .iter()
        .find(|event| event.ex_dividend_date_stock.is_some())
        .expect("應取得只有除權日的事件");
    assert_eq!(dirty.ex_dividend_date_cash, None);
    assert_eq!(
        dirty.ex_dividend_date_stock,
        NaiveDate::from_ymd_opt(1991, 3, 1)
    );

    // 兩個日期都是髒值的事件無法定位，整筆略過。
    assert!(
        !mine.iter().any(|event| event.stock_symbol == FAKE_B),
        "沒有任何可解析日期的事件不得回傳"
    );

    // since 之後才生效：1991 的事件仍在，1990 的已被濾掉。
    let later = repo
        .fetch_dividend_events_since(NaiveDate::from_ymd_opt(1991, 1, 1).unwrap())
        .await
        .expect("fetch_dividend_events_since later");
    let later_mine: Vec<&DividendEvent> = later
        .iter()
        .filter(|event| event.stock_symbol == FAKE_A)
        .collect();
    assert_eq!(later_mine.len(), 1);
    assert_eq!(later_mine[0].stock_dividend, dec!(1));

    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_anomaly_detection_ignores_jumps_explained_by_dividends() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_anomaly_detection_ignores_jumps_explained_by_dividends：無資料庫連接");
        return;
    }
    seed().await;
    insert_listing_history(FAKE_A).await;
    let repo = PgCagrSourceRepository::new();

    let events = repo
        .fetch_anomaly_events(day(2), day(8))
        .await
        .expect("fetch_anomaly_events");
    let mine: Vec<NaiveDate> = events
        .iter()
        .filter(|(symbol, _)| symbol == FAKE_A)
        .map(|(_, date)| *date)
        .collect();

    // 01-03 跳 60% 但當天有登錄除息日 → 可解釋，不標記；
    // 01-05 跳 87.5% 且無對應除權息 → 標記；
    // 01-08 僅跳 3.3%，未達 30% 門檻。
    assert_eq!(mine, vec![day(5)]);

    // 分桶併發掃描與不分桶的單一語句結果必須相同。
    let mut unbucketed = fetch_anomaly_bucket(day(2), day(8), (1, 0))
        .await
        .expect("不分桶掃描應成功");
    let mut bucketed = events.clone();
    unbucketed.sort();
    bucketed.sort();
    assert_eq!(bucketed, unbucketed);

    // 事件帶日期回傳，呼叫端才能依期間各自判定：縮小區間後該事件應消失。
    let narrowed = repo
        .fetch_anomaly_events(day(5), day(8))
        .await
        .expect("fetch_anomaly_events narrowed");
    assert!(
        !narrowed
            .iter()
            .any(|(symbol, date)| symbol == FAKE_A && *date == day(5)),
        "區間起點之後才計算跳動，起點當天不應再被標記"
    );

    cleanup().await;
}

/// 上市初期（前 5 筆報價）的跳動不算異常；公司行動生效日沒有報價時，
/// 只要落在前一筆報價之後、這一筆（含）之前就算已解釋。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_anomaly_detection_skips_listing_days_and_matches_actions_between_quotes() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!(
            "跳過 test_anomaly_detection_skips_listing_days_and_matches_actions_between_quotes：無資料庫連接"
        );
        return;
    }
    cleanup().await;
    insert_stock(FAKE_C, false).await;
    // 01-03 是上市第 2 筆、漲一倍：上市初期，不算異常。
    for (d, close) in [
        (2, dec!(10)),
        (3, dec!(20)),
        (4, dec!(20)),
        (5, dec!(20)),
        (8, dec!(20)),
    ] {
        insert_quote(FAKE_C, day(d), close).await;
    }
    // 01-09 生效的減資當天沒有成交，只有沿用停牌前價格的零量補值列（1538 2024-12-30 的情況），
    // 01-10 才有成交、價格跳 1.5 倍：補值列不算前一筆報價，仍視為已解釋。
    insert_corporate_action(FAKE_C, day(9), dec!(0.4)).await;
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
           VALUES ($1, $2, 20, 0)"#,
    )
    .bind(day(9))
    .bind(FAKE_C)
    .execute(database::get_connection())
    .await
    .expect("插入零量補值列");
    insert_quote(FAKE_C, day(10), dec!(50)).await;
    // 01-12 再跳一倍，沒有任何事件：異常。
    insert_quote(FAKE_C, day(12), dec!(100)).await;

    let events = PgCagrSourceRepository::new()
        .fetch_anomaly_events(day(1), day(31))
        .await
        .expect("fetch_anomaly_events");
    cleanup().await;

    let mine: Vec<NaiveDate> = events
        .iter()
        .filter(|(symbol, _)| symbol == FAKE_C)
        .map(|(_, date)| *date)
        .collect();
    assert_eq!(mine, vec![day(12)]);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_corporate_actions_are_fetched_and_suppress_anomaly_flags() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!(
            "跳過 test_corporate_actions_are_fetched_and_suppress_anomaly_flags：無資料庫連接"
        );
        return;
    }
    seed().await;
    insert_listing_history(FAKE_A).await;
    let repo = PgCagrSourceRepository::new();

    // 尚未登錄時，01-05 那次 87.5% 的跳動會被標記為異常。
    let before = repo
        .fetch_anomaly_events(day(2), day(8))
        .await
        .expect("fetch_anomaly_events");
    assert!(
        before
            .iter()
            .any(|(symbol, date)| symbol == FAKE_A && *date == day(5))
    );

    insert_corporate_action(FAKE_A, day(5), dec!(0.5)).await;

    let actions = repo
        .fetch_corporate_actions_since(day(1))
        .await
        .expect("fetch_corporate_actions_since");
    let mine: Vec<&CorporateAction> = actions
        .iter()
        .filter(|action| action.stock_symbol == FAKE_A)
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].effective_date, day(5));
    assert_eq!(mine[0].share_ratio, dec!(0.5));

    // 登錄之後同一天不得再被視為異常——事件已建模，旗標應該熄滅。
    let after = repo
        .fetch_anomaly_events(day(2), day(8))
        .await
        .expect("fetch_anomaly_events again");
    assert!(
        !after
            .iter()
            .any(|(symbol, date)| symbol == FAKE_A && *date == day(5)),
        "已登錄的分割不該再被標記為疑似異常"
    );

    // since 之後才生效者才回傳。
    let later = repo
        .fetch_corporate_actions_since(day(5))
        .await
        .expect("fetch_corporate_actions_since later");
    assert!(!later.iter().any(|action| action.stock_symbol == FAKE_A));

    cleanup().await;
}

/// 驗證髒值一律回傳 `None`，合法日期正常解析。
#[test]
fn test_parse_ex_dividend_date() {
    assert_eq!(
        parse_ex_dividend_date("2024-07-11"),
        NaiveDate::from_ymd_opt(2024, 7, 11)
    );
    assert_eq!(
        parse_ex_dividend_date(" 2024-07-11 "),
        NaiveDate::from_ymd_opt(2024, 7, 11)
    );

    // 以下皆為資料庫中實際出現過的髒值。
    assert_eq!(parse_ex_dividend_date("-"), None);
    assert_eq!(parse_ex_dividend_date("尚未公布"), None);
    assert_eq!(parse_ex_dividend_date(""), None);
    assert_eq!(parse_ex_dividend_date("1.39%"), None);
    assert_eq!(parse_ex_dividend_date("2024/07/11"), None);
}
