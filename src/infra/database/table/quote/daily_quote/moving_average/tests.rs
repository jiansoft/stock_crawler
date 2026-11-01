//! 均線與年內統計的測試：依日期回填、空批次防呆與整段重算對照逐日回填。

use chrono::NaiveDate;
use rust_decimal::Decimal;

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
