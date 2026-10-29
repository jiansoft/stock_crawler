//! 相對強度計分母體（近 5 個交易日有收盤）的資料庫語意整合測試。

use super::*;

/// 母體看「資料日前 5 個交易日內有沒有收盤」，不看目前的下市旗標：RG 每天有日線（構成交易日曆）；
/// RH 已標記下市但資料日仍有收盤（終止上市後轉上櫃），列入；RI 沒有標記下市，但最後一筆收盤在
/// 資料日前第 5 個交易日（已超過 5 個交易日沒有收盤），不列入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn relative_strength_universe_uses_recent_quotes() {
    let Some(pool) = pool_or_skip("relative_strength_universe_uses_recent_quotes").await else {
        return;
    };
    let symbols = ["79979RG", "79979RH", "79979RI"];
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_relative_strength WHERE stock_symbol = ANY($1)",
            "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(&symbols[..]).execute(pool).await;
        }
    };
    let dates = weekdays(241);
    let end = dates[dates.len() - 1];
    cleanup().await;
    for (symbol, suspended) in [("79979RG", false), ("79979RH", true), ("79979RI", false)] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing")
               VALUES ($1, $1, $1, 4, 24, $2)"#,
        )
        .bind(symbol)
        .bind(suspended)
        .execute(pool)
        .await
        .expect("種入 stocks");
    }
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
           SELECT d, '79979RG', 50 FROM UNNEST($1::date[]) AS d
           UNION ALL SELECT $2, '79979RH', 20
           UNION ALL SELECT $3, '79979RI', 30"#,
    )
    .bind(&dates)
    .bind(end)
    .bind(dates[dates.len() - 6])
    .execute(pool)
    .await
    .expect("種入日線");
    for symbol in symbols {
        for horizon in [60_usize, 120, 240] {
            sqlx::query(
                "INSERT INTO daily_forward_return AS t (date, stock_symbol, return_60d, return_120d, return_240d)
                 VALUES ($1, $2, CASE WHEN $3 = 60 THEN 1 END, CASE WHEN $3 = 120 THEN 1 END,
                         CASE WHEN $3 = 240 THEN 1 END)
                 ON CONFLICT (date, stock_symbol) DO UPDATE
                 SET return_60d = COALESCE(EXCLUDED.return_60d, t.return_60d),
                     return_120d = COALESCE(EXCLUDED.return_120d, t.return_120d),
                     return_240d = COALESCE(EXCLUDED.return_240d, t.return_240d)",
            )
            .bind(dates[dates.len() - 1 - horizon])
            .bind(symbol)
            .bind(i32::try_from(horizon).expect("天期很小"))
            .execute(pool)
            .await
            .expect("種入報酬");
        }
    }

    crate::app::calculation::relative_strength::calculate_relative_strength(end)
        .await
        .expect("儲存每日排名");
    let (status, json) = get(&format!("/api/v1/market/relative-strength?date={end}")).await;
    cleanup().await;

    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["universe"], 2, "{json}");
    let listed: Vec<&str> = json["items"]
        .as_array()
        .expect("items 是陣列")
        .iter()
        .filter_map(|item| item["stock_symbol"].as_str())
        .collect();
    assert_eq!(listed, ["79979RG", "79979RH"]);
}
