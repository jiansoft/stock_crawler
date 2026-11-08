//! 除權息日漲跌修正的 PostgreSQL 實作（[`ExRightChangeRepository`]）的測試。

use rust_decimal_macros::dec;

use super::*;
use crate::infra::database::table::quote::daily_stock_price_stats::DailyStockPriceStats;

type StatsRow = (i32, i32, i32, i32);

async fn stats_rows(date: NaiveDate) -> Vec<StatsRow> {
    sqlx::query_as(
        "SELECT stock_exchange_market_id, stocks_up, stocks_down, stocks_unchanged
         FROM daily_stock_price_stats WHERE date = $1 ORDER BY 1",
    )
    .bind(date)
    .fetch_all(database::get_connection())
    .await
    .expect("讀回統計")
}

/// 以基準價重算漲跌為 0 的除權息日；統計表依修正結果調整後，與從頭重算的結果相同。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn apply_references_fixes_quotes_and_stats() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 apply_references_fixes_quotes_and_stats：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let date = NaiveDate::from_ymd_opt(1991, 7, 15).expect("有效日期");
    let symbols = [
        "79979XA", "79979XB", "79979XC", "79979XD", "79979XE", "79979XF",
    ];
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
            "DELETE FROM estimate WHERE security_code = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            sqlx::query(sql)
                .bind(symbols)
                .execute(pool)
                .await
                .expect("清理測試資料");
        }
        sqlx::query("DELETE FROM daily_stock_price_stats WHERE date = $1")
            .bind(date)
            .execute(pool)
            .await
            .expect("清理統計");
    };
    cleanup().await;

    // (代號, 市場, 收盤, 原漲跌, 有估價)：XA 除息後上漲、XB 上櫃下跌、XC 收平、
    // XD 原本就有漲跌、XE 沒有估價（不在統計取樣內）、XF 沒有基準價。
    for (symbol, market, close, change, estimated) in [
        ("79979XA", 2, dec!(9.5), dec!(0), true),
        ("79979XB", 4, dec!(8), dec!(0), true),
        ("79979XC", 2, dec!(9), dec!(0), true),
        ("79979XD", 2, dec!(9.5), dec!(0.5), true),
        ("79979XE", 2, dec!(9.5), dec!(0), false),
        ("79979XF", 2, dec!(9.5), dec!(0), true),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, $2, 1)"#,
        )
        .bind(symbol)
        .bind(market)
        .execute(pool)
        .await
        .expect("寫入測試股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "Change", "ChangeRange")
               VALUES ($1, $2, $3, $4, CASE WHEN $4 = 0 THEN 0 ELSE $4 / ($3 - $4) * 100 END)"#,
        )
        .bind(date)
        .bind(symbol)
        .bind(close)
        .bind(change)
        .execute(pool)
        .await
        .expect("寫入測試日線");
        if estimated {
            sqlx::query("INSERT INTO estimate (security_code, date) VALUES ($1, $2)")
                .bind(symbol)
                .bind(date)
                .execute(pool)
                .await
                .expect("寫入測試估價");
        }
    }
    DailyStockPriceStats::upsert(date, &mut None)
        .await
        .expect("寫入統計");
    assert_eq!(
        stats_rows(date).await,
        [(0, 1, 0, 4), (2, 1, 0, 3), (4, 0, 0, 1)]
    );

    let reference = |symbol: &str| ExRightReference {
        stock_symbol: symbol.to_owned(),
        date,
        reference_price: dec!(9),
    };
    let references: Vec<ExRightReference> = [
        "79979XA", "79979XA", "79979XB", "79979XC", "79979XD", "79979XE",
    ]
    .into_iter()
    .map(reference)
    .collect();
    let repo = PgQuoteRepository::new();
    let updated = repo
        .apply_ex_right_references(&references, true)
        .await
        .expect("修正成功");
    assert_eq!(updated, 4);

    let quotes: Vec<(String, Decimal, Decimal)> = sqlx::query_as(
        r#"SELECT stock_symbol, "Change", "ChangeRange" FROM "DailyQuotes"
           WHERE stock_symbol = ANY($1) AND "Date" = $2 ORDER BY 1"#,
    )
    .bind(symbols)
    .bind(date)
    .fetch_all(pool)
    .await
    .expect("讀回日線");
    assert_eq!(
        quotes,
        [
            ("79979XA".to_owned(), dec!(0.5), dec!(5.5556)),
            ("79979XB".to_owned(), dec!(-1), dec!(-11.1111)),
            ("79979XC".to_owned(), dec!(0), dec!(0)),
            ("79979XD".to_owned(), dec!(0.5), dec!(5.5556)),
            ("79979XE".to_owned(), dec!(0.5), dec!(5.5556)),
            ("79979XF".to_owned(), dec!(0), dec!(0)),
        ]
    );
    let adjusted = stats_rows(date).await;
    assert_eq!(adjusted, [(0, 2, 1, 2), (2, 2, 0, 2), (4, 0, 1, 0)]);
    DailyStockPriceStats::upsert(date, &mut None)
        .await
        .expect("重算統計");
    assert_eq!(stats_rows(date).await, adjusted, "調整結果應與從頭重算相同");

    // 再跑一次（不調整統計）：只剩收平的 XC 仍是 0，數值不變；空清單不動資料庫。
    assert_eq!(
        repo.apply_ex_right_references(&references, false)
            .await
            .expect("重跑成功"),
        1
    );
    assert_eq!(
        repo.apply_ex_right_references(&[], true)
            .await
            .expect("空清單"),
        0
    );
    assert_eq!(stats_rows(date).await, adjusted);

    cleanup().await;
}
