//! 技術指標日線與技術訊號倉儲的整合測試。

use super::*;
use crate::domain::quote::technical::{SignalKind, TechnicalSignal};

fn signal(symbol: &str, kind: SignalKind) -> StockTechnicalSignal {
    StockTechnicalSignal {
        date: NaiveDate::from_ymd_opt(1991, 9, 2).expect("有效日期"),
        stock_symbol: symbol.to_owned(),
        signal: TechnicalSignal {
            indicator: "kd",
            kind,
            description: format!("{symbol} 測試訊號"),
        },
    }
}

/// 只列出當日有成交、未暫停的上市櫃股票；寫入會整日覆蓋舊訊號。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn signal_symbols_and_replace_round_trip() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 signal_symbols_and_replace_round_trip：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let date = NaiveDate::from_ymd_opt(1991, 9, 2).expect("有效日期");
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979S_'"#,
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979S_'",
            "DELETE FROM daily_technical_signal WHERE date = '1991-09-02'",
        ] {
            sqlx::query(sql).execute(pool).await.expect("清理測試資料");
        }
    };
    cleanup().await;

    // A 上市有成交、B 上櫃但收盤價 0、C 暫停交易、D 市場別 0。
    for (symbol, market, suspended, close) in [
        ("79979SA", 2, false, 10),
        ("79979SB", 4, false, 0),
        ("79979SC", 2, true, 10),
        ("79979SD", 0, false, 10),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, "SuspendListing", stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, $2, $3, 0)"#,
        )
        .bind(symbol)
        .bind(suspended)
        .bind(market)
        .execute(pool)
        .await
        .expect("寫入測試股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" (stock_symbol, "Date", "ClosingPrice", year, month, day)
               VALUES ($1, $2, $3, 1991, 9, 2)"#,
        )
        .bind(symbol)
        .bind(date)
        .bind(rust_decimal::Decimal::from(close))
        .execute(pool)
        .await
        .expect("寫入測試報價");
    }

    let repo = PgQuoteRepository::new();
    let symbols = repo.fetch_signal_symbols(date).await.expect("查詢代號");
    let ours: Vec<&String> = symbols.iter().filter(|s| s.starts_with("79979S")).collect();
    assert_eq!(ours, ["79979SA"]);

    let first = [
        signal("79979SA", SignalKind::GoldenCross),
        signal("79979SA", SignalKind::Overbought),
    ];
    repo.replace_signals(date, &first)
        .await
        .expect("第一次寫入");
    let read = repo
        .fetch_signals(date, &["79979SA".to_owned(), "79979SB".to_owned()])
        .await
        .expect("讀回訊號");
    assert_eq!(read, first);
    sqlx::query(
        "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
         VALUES ($1, '79979SA', 'unknown', 'golden_cross', '舊資料')",
    )
    .bind(date)
    .execute(pool)
    .await
    .expect("寫入無法辨識的訊號");
    assert_eq!(
        repo.fetch_signals(date, &["79979SA".to_owned()])
            .await
            .expect("讀回訊號")
            .len(),
        2,
        "無法辨識的來源應略過"
    );
    // 相對強度穿越訊號由每日相對強度負責寫入，整天替換技術訊號時要保留。
    sqlx::query(
        "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
         VALUES ($1, '79979SA', 'rs80', 'cross_above', '相對強度 76 → 82 分，突破 80 分')",
    )
    .bind(date)
    .execute(pool)
    .await
    .expect("寫入相對強度訊號");
    repo.replace_signals(date, &first[1..])
        .await
        .expect("覆蓋寫入");
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT stock_symbol, indicator, kind, description FROM daily_technical_signal
         WHERE date = $1 ORDER BY indicator",
    )
    .bind(date)
    .fetch_all(pool)
    .await
    .expect("讀回訊號");
    assert_eq!(
        rows,
        [
            (
                "79979SA".to_owned(),
                "kd".to_owned(),
                "overbought".to_owned(),
                "79979SA 測試訊號".to_owned()
            ),
            (
                "79979SA".to_owned(),
                "rs80".to_owned(),
                "cross_above".to_owned(),
                "相對強度 76 → 82 分，突破 80 分".to_owned()
            )
        ]
    );

    repo.replace_signals(date, &[]).await.expect("清空");
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM daily_technical_signal WHERE date = $1")
            .bind(date)
            .fetch_one(pool)
            .await
            .expect("計數");
    assert_eq!(count, 1, "只剩相對強度訊號");

    cleanup().await;
}

/// 除息、除權與公司行動依股票與日期合併；同日重複的股利取最大值，非日期的除權息日略過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn price_adjustments_merge_dividends_and_corporate_actions() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 price_adjustments_merge_dividends_and_corporate_actions：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM dividend WHERE security_code LIKE '79979J_'",
            "DELETE FROM corporate_action WHERE stock_symbol LIKE '79979J_'",
        ] {
            sqlx::query(sql).execute(pool).await.expect("清理測試資料");
        }
    };
    cleanup().await;

    // JA：同日月配與季配重複（取一筆）、另一年同日除權息、日期為「-」的年度彙總列。
    for (symbol, year, quarter, cash, stock, date1, date2) in [
        ("79979JA", 1991, "M04", "0.14", "0", "1991-07-16", "-"),
        ("79979JA", 1991, "Q2", "0.14", "0", "1991-07-16", "-"),
        (
            "79979JA",
            1992,
            "",
            "1.5",
            "0.5",
            "1992-08-01",
            "1992-08-01",
        ),
        ("79979JA", 1993, "", "2", "0", "-", "-"),
        ("79979JB", 1991, "", "0", "1", "-", "1991-09-02"),
    ] {
        sqlx::query(
            r#"INSERT INTO dividend (security_code, year, quarter, cash_dividend, stock_dividend,
                                     "ex-dividend_date1", "ex-dividend_date2")
               VALUES ($1, $2, $3, $4::numeric, $5::numeric, $6, $7)"#,
        )
        .bind(symbol)
        .bind(year)
        .bind(quarter)
        .bind(cash)
        .bind(stock)
        .bind(date1)
        .bind(date2)
        .execute(pool)
        .await
        .expect("寫入測試股利");
    }
    sqlx::query(
        "INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio)
         VALUES ('79979JB', '1991-10-01', 'capital_reduction', 0.7)",
    )
    .execute(pool)
    .await
    .expect("寫入測試公司行動");

    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("有效日期");
    let map = PgQuoteRepository::new()
        .fetch_price_adjustments(&[
            "79979JA".to_owned(),
            "79979JB".to_owned(),
            "79979JC".to_owned(),
        ])
        .await
        .expect("查詢事件");
    cleanup().await;

    let adjustment = |date, cash, stock, share_ratio| PriceAdjustment {
        date,
        cash_dividend: cash,
        stock_dividend: stock,
        share_ratio,
    };
    assert_eq!(map.len(), 2, "{map:?}");
    assert_eq!(
        map["79979JA"],
        [
            adjustment(day(1991, 7, 16), 0.14, 0.0, 1.0),
            adjustment(day(1992, 8, 1), 1.5, 0.5, 1.0),
        ]
    );
    assert_eq!(
        map["79979JB"],
        [
            adjustment(day(1991, 9, 2), 0.0, 1.0, 1.0),
            adjustment(day(1991, 10, 1), 0.0, 0.0, 0.7),
        ]
    );
}

/// 只統計指定來源、已滿 20 日的訊號；依來源與種類分組。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn signal_outcomes_group_by_indicator_and_kind() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 signal_outcomes_group_by_indicator_and_kind：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let day = |d| NaiveDate::from_ymd_opt(1989, 3, d).expect("有效日期");
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_technical_signal WHERE stock_symbol LIKE '79979O_'",
            "DELETE FROM daily_forward_return WHERE stock_symbol LIKE '79979O_'",
        ] {
            sqlx::query(sql).execute(pool).await.expect("清理測試資料");
        }
    };
    cleanup().await;

    // OA、OB 黃金交叉 +10%、−2%；OC 死亡交叉未滿 20 日；OD 是其他來源；OE 在期間外。
    for (d, symbol, indicator, kind, value) in [
        (3, "79979OA", "kd", "golden_cross", Some(10)),
        (3, "79979OB", "kd", "golden_cross", Some(-2)),
        (3, "79979OC", "kd", "death_cross", None),
        (3, "79979OD", "rsi14", "overbought", Some(5)),
        (28, "79979OE", "kd", "golden_cross", Some(50)),
    ] {
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, $2, $3, $4, '測試')",
        )
        .bind(day(d))
        .bind(symbol)
        .bind(indicator)
        .bind(kind)
        .execute(pool)
        .await
        .expect("寫入測試訊號");
        sqlx::query(
            "INSERT INTO daily_forward_return (date, stock_symbol, return_20d) VALUES ($1, $2, $3)",
        )
        .bind(day(d))
        .bind(symbol)
        .bind(value.map(rust_decimal::Decimal::from))
        .execute(pool)
        .await
        .expect("寫入測試報酬");
    }

    let outcomes = PgQuoteRepository::new()
        .fetch_signal_outcomes(day(1), day(20), &["kd"])
        .await
        .expect("統計訊號表現");
    cleanup().await;
    assert_eq!(
        outcomes,
        [SignalOutcome {
            indicator: "kd".to_owned(),
            kind: "golden_cross".to_owned(),
            samples: 2,
            avg_return: 4.0,
            win_rate: 0.5,
        }]
    );
}
