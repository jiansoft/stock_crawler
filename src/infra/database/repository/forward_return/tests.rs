//! 未來報酬倉儲與大盤彙總重算的測試。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;

/// (代號, 5 日, 20 日, 60 日, 240 日報酬)。
type ReturnRow = (
    String,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
);

/// 交易日由新到舊；報酬依除息與減資還原，不同天期寫在各自欄位、互不覆蓋。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn forward_returns_adjust_for_events() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 forward_returns_adjust_for_events：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let day = |d| NaiveDate::from_ymd_opt(1991, 8, d).expect("有效日期");
    let symbols = ["79979FA", "79979FB", "79979FC", "79979FD"];
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
            "DELETE FROM dividend WHERE security_code = ANY($1)",
            "DELETE FROM corporate_action WHERE stock_symbol = ANY($1)",
            "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
            "DELETE FROM daily_technical_signal WHERE stock_symbol = ANY($1)",
        ] {
            sqlx::query(sql)
                .bind(symbols)
                .execute(pool)
                .await
                .expect("清理測試資料");
        }
    };
    cleanup().await;

    // FA：8/2 收 100，8/6 除息 10 元（前收 100 → 參考價 90），8/8 收 99 → 還原報酬 +10%。
    // FB：8/6 減資五成（1 股變 0.5 股），8/8 收 220 → 前收 100 × 2 = 200，報酬 +10%。
    // FC：基準日沒有成交，不寫入；FD 是興櫃，不寫入。
    for (symbol, market, quotes) in [
        (
            "79979FA",
            2,
            vec![(2, dec!(100)), (5, dec!(100)), (6, dec!(95)), (8, dec!(99))],
        ),
        (
            "79979FB",
            4,
            vec![(2, dec!(100)), (5, dec!(100)), (8, dec!(220))],
        ),
        ("79979FC", 2, vec![(5, dec!(10)), (8, dec!(11))]),
        ("79979FD", 5, vec![(2, dec!(10)), (8, dec!(11))]),
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
        for (d, close) in quotes {
            sqlx::query(
                r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES ($1, $2, $3)"#,
            )
            .bind(day(d))
            .bind(symbol)
            .bind(close)
            .execute(pool)
            .await
            .expect("寫入測試日線");
        }
    }
    sqlx::query(
        r#"INSERT INTO dividend (security_code, year, quarter, cash_dividend, "ex-dividend_date1", "ex-dividend_date2")
           VALUES ('79979FA', 1991, '', 10, '1991-08-06', '-'),
                  ('79979FA', 1990, '', 3, '1991-08-01', '-')"#,
    )
    .execute(pool)
    .await
    .expect("寫入測試股利");
    sqlx::query(
        "INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio)
         VALUES ('79979FB', '1991-08-06', 'capital_reduction', 0.5)",
    )
    .execute(pool)
    .await
    .expect("寫入測試公司行動");

    let repo = PgQuoteRepository::new();
    let dates = repo
        .fetch_trading_dates(day(8), 3)
        .await
        .expect("查詢交易日");
    assert_eq!(dates, [day(8), day(6), day(5)]);
    // 只有相對強度穿越訊號時，技術訊號仍算缺漏。
    sqlx::query(
        "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
         VALUES ($1, '79979FA', 'rs50', 'cross_above', '相對強度 40 → 60 分，突破 50 分')",
    )
    .bind(day(8))
    .execute(pool)
    .await
    .expect("寫入相對強度訊號");
    let before = repo
        .count_derived(day(8), day(2))
        .await
        .expect("寫入前的筆數");
    assert_eq!((before.signals, before.forward_returns), (0, 0));

    assert_eq!(
        repo.upsert_forward_returns(day(2), day(8), 5)
            .await
            .expect("寫入 5 日"),
        2
    );
    repo.upsert_forward_returns(day(2), day(5), 20)
        .await
        .expect("寫入 20 日");
    repo.upsert_forward_returns(day(2), day(8), 240)
        .await
        .expect("寫入 240 日");
    // 重算 5 日不影響 20 日欄位。
    repo.upsert_forward_returns(day(2), day(8), 5)
        .await
        .expect("重算 5 日");

    let counts = repo
        .count_derived(day(8), day(2))
        .await
        .expect("寫入後的筆數");
    assert_eq!(counts.forward_returns, 2);
    let rows: Vec<ReturnRow> = sqlx::query_as(
        "SELECT stock_symbol, return_5d, return_20d, return_60d, return_240d FROM daily_forward_return
             WHERE stock_symbol = ANY($1) ORDER BY 1",
    )
    .bind(symbols)
    .fetch_all(pool)
    .await
    .expect("讀回報酬");
    cleanup().await;
    assert_eq!(
        rows,
        [
            (
                "79979FA".to_owned(),
                Some(dec!(10)),
                Some(dec!(0)),
                None,
                Some(dec!(10))
            ),
            (
                "79979FB".to_owned(),
                Some(dec!(10)),
                Some(dec!(0)),
                None,
                Some(dec!(10))
            ),
        ]
    );
}
/// 每日大盤彙總與逐列 `AVG`（原本訊號績效的 bench 與 benchmark）完全相同：每天的平均、
/// 整段的筆數、平均與上漲比例，三個市場口徑都比到 numeric 的文字；報酬清掉後彙總列也跟著刪掉。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn market_forward_returns_match_row_averages() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 market_forward_returns_match_row_averages：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let day = |d| NaiveDate::from_ymd_opt(1981, 3, d).expect("日期合法");
    let symbols = ["79976A", "79976B", "79976C", "79976D"];
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            sqlx::query(sql)
                .bind(&symbols[..])
                .execute(pool)
                .await
                .unwrap();
        }
        refresh_market_forward_returns_between(day(1), day(31))
            .await
            .unwrap();
    };
    cleanup().await;
    for (symbol, market) in [("79976A", 2), ("79976B", 2), ("79976C", 4), ("79976D", 4)] {
        sqlx::query(r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id) VALUES ($1, $1, $1, $2, 24)"#)
            .bind(symbol).bind(market).execute(pool).await.unwrap();
    }
    // 小數位、負值、NULL 混在一起；03-04 全部 5 日報酬都是 NULL。
    let returns: [(u32, &str, Option<Decimal>, Option<Decimal>); 9] = [
        (2, "79976A", Some(dec!(1.2345)), Some(dec!(-3.3333))),
        (2, "79976B", Some(dec!(-0.5)), None),
        (2, "79976C", Some(dec!(2.0001)), Some(dec!(7.1))),
        (2, "79976D", None, Some(dec!(0))),
        (3, "79976A", Some(dec!(0)), Some(dec!(1.1111))),
        (3, "79976C", Some(dec!(-9.8765)), Some(dec!(2.2222))),
        (3, "79976D", Some(dec!(4.4444)), None),
        (4, "79976A", None, Some(dec!(5.5))),
        (4, "79976B", None, Some(dec!(-1.25))),
    ];
    for (d, symbol, r5, r20) in returns {
        sqlx::query("INSERT INTO daily_forward_return (date, stock_symbol, return_5d, return_20d, return_240d) VALUES ($1, $2, $3, $4, $3)")
            .bind(day(d)).bind(symbol).bind(r5).bind(r20).execute(pool).await.unwrap();
    }
    assert_eq!(
        refresh_market_forward_returns_between(day(1), day(31))
            .await
            .unwrap(),
        3
    );

    type Daily = Vec<(NaiveDate, Option<String>, Option<String>, Option<String>)>;
    type Period = (
        i64,
        Option<String>,
        Option<String>,
        i64,
        Option<String>,
        Option<String>,
    );
    for market in [0, 2, 4] {
        let old_daily: Daily = sqlx::query_as(
            r#"SELECT r.date, AVG(r.return_5d)::text, AVG(r.return_20d)::text, AVG(r.return_240d)::text
               FROM daily_forward_return r JOIN stocks s ON s.stock_symbol = r.stock_symbol
               WHERE r.date BETWEEN $1 AND $2 AND ($3 = 0 OR s.stock_exchange_market_id = $3)
               GROUP BY r.date ORDER BY 1"#,
        )
        .bind(day(1)).bind(day(31)).bind(market).fetch_all(pool).await.unwrap();
        let new_daily: Daily = sqlx::query_as(
            r#"SELECT date, (sum5 / NULLIF(n5, 0))::text, (sum20 / NULLIF(n20, 0))::text, (sum240 / NULLIF(n240, 0))::text
               FROM daily_market_forward_return
               WHERE date BETWEEN $1 AND $2 AND market_id = $3 ORDER BY 1"#,
        )
        .bind(day(1)).bind(day(31)).bind(market).fetch_all(pool).await.unwrap();
        assert_eq!(new_daily, old_daily, "每日平均 market={market}");

        let old_period: Period = sqlx::query_as(
            r#"SELECT COUNT(r.return_5d), AVG(r.return_5d)::text,
                      AVG(CASE WHEN r.return_5d > 0 THEN 1.0 WHEN r.return_5d IS NOT NULL THEN 0.0 END)::text,
                      COUNT(r.return_20d), AVG(r.return_20d)::text,
                      AVG(CASE WHEN r.return_20d > 0 THEN 1.0 WHEN r.return_20d IS NOT NULL THEN 0.0 END)::text
               FROM daily_forward_return r JOIN stocks s ON s.stock_symbol = r.stock_symbol
               WHERE r.date BETWEEN $1 AND $2 AND ($3 = 0 OR s.stock_exchange_market_id = $3)"#,
        )
        .bind(day(1)).bind(day(31)).bind(market).fetch_one(pool).await.unwrap();
        let new_period: Period = sqlx::query_as(
            r#"SELECT COALESCE(SUM(n5), 0)::bigint, (SUM(sum5) / NULLIF(SUM(n5), 0))::text,
                      (SUM(win5) / NULLIF(SUM(n5), 0))::text,
                      COALESCE(SUM(n20), 0)::bigint, (SUM(sum20) / NULLIF(SUM(n20), 0))::text,
                      (SUM(win20) / NULLIF(SUM(n20), 0))::text
               FROM daily_market_forward_return
               WHERE date BETWEEN $1 AND $2 AND market_id = $3"#,
        )
        .bind(day(1))
        .bind(day(31))
        .bind(market)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(new_period, old_period, "整段統計 market={market}");
    }

    cleanup().await;
    let left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM daily_market_forward_return WHERE date BETWEEN $1 AND $2",
    )
    .bind(day(1))
    .bind(day(31))
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(left, 0, "報酬清掉後彙總列應跟著刪除");
}
