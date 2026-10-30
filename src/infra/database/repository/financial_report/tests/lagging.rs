//! `fetch_lagging_statements`：損益表落後官方季報的股票。

use super::*;

/// 落後判斷用的假代號：LA 官方今天公布 Q2、Yahoo 只到 Q1；LB 官方今天公布 Q2、BigGo 已有 Q2；
/// LC 官方 Q2 是 60 天前公布；LD 官方 10 天前公布 Q1（另有年度列）、損益表完全沒有；
/// LE 已下市、LF 興櫃，官方今天公布 Q2、損益表沒有。
const LAGGING_SYMBOLS: [&str; 6] = [
    "79979LA", "79979LB", "79979LC", "79979LD", "79979LE", "79979LF",
];

async fn cleanup_lagging() {
    for (table, column) in [
        ("financial_statement", "security_code"),
        ("income_statement", "stock_symbol"),
        ("stocks", "stock_symbol"),
    ] {
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE {column} = ANY($1)"
        )))
        .bind(&LAGGING_SYMBOLS[..])
        .execute(database::get_connection())
        .await;
    }
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_lagging_statements() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_lagging_statements：無資料庫連接");
        return;
    }
    cleanup_lagging().await;

    for (symbol, market_id, suspended) in [
        ("79979LA", 2, false),
        ("79979LB", 2, false),
        ("79979LC", 4, false),
        ("79979LD", 4, false),
        ("79979LE", 2, true),
        ("79979LF", 5, false),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing")
               VALUES ($1, $1, $1, $2, 1, $3)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .bind(suspended)
        .execute(database::get_connection())
        .await
        .expect("insert stock");
    }
    for (symbol, quarter, days_ago) in [
        ("79979LA", "Q1", 0),
        ("79979LA", "Q2", 0),
        ("79979LB", "Q2", 0),
        ("79979LC", "Q2", 60),
        ("79979LD", "Q1", 10),
        ("79979LD", "A", 10),
        ("79979LE", "Q2", 0),
        ("79979LF", "Q2", 0),
    ] {
        sqlx::query(
            "INSERT INTO financial_statement (security_code, \"year\", quarter, created_time)              VALUES ($1, $2, $3, now() - make_interval(days => $4))",
        )
        .bind(symbol)
        .bind(i64::from(FAKE_YEAR))
        .bind(quarter)
        .bind(days_ago)
        .execute(database::get_connection())
        .await
        .expect("insert official statement");
    }
    let statement = |symbol: &str, quarter: Quarter, source: &str| IncomeStatement {
        stock_symbol: symbol.to_owned(),
        source: source.to_owned(),
        ..income_statement(StatementPeriod::Single(quarter), 1_000)
    };
    let repo = PgFinancialReportRepository::new();
    repo.save_income_statements(&[
        statement("79979LA", Quarter::Q1, "yahoo"),
        statement("79979LB", Quarter::Q2, "biggo"),
    ])
    .await
    .expect("save income statements");

    let fake = |statements: Vec<LaggingStatement>| -> Vec<LaggingStatement> {
        statements
            .into_iter()
            .filter(|statement| LAGGING_SYMBOLS.contains(&statement.stock_symbol.as_str()))
            .collect()
    };
    let recent = fake(
        repo.fetch_lagging_statements(30, 0)
            .await
            .expect("fetch lagging"),
    );
    let symbols: Vec<&str> = recent.iter().map(|s| s.stock_symbol.as_str()).collect();
    assert_eq!(symbols, ["79979LA", "79979LD"]);
    assert_eq!(
        (recent[0].fiscal_year, recent[0].quarter.as_str()),
        (FAKE_YEAR, "Q2")
    );

    // 公布滿 7 天才算：只剩 10 天前公布的 LD。
    let stale = fake(
        repo.fetch_lagging_statements(30, 7)
            .await
            .expect("fetch stale"),
    );
    // SQL 以台北時間（`AT TIME ZONE 'Asia/Taipei'`）換算公布日；預期值也要用 UTC+8，不能用執行機器的
    // 本地時區（CI 是 UTC，台北 00:00～08:00 之間兩邊會差一天）。
    let taipei = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 合法");
    let ten_days_ago =
        (chrono::Utc::now().with_timezone(&taipei) - chrono::Duration::days(10)).date_naive();
    assert_eq!(
        stale,
        [LaggingStatement {
            stock_symbol: "79979LD".to_owned(),
            fiscal_year: FAKE_YEAR,
            quarter: "Q1".to_owned(),
            published_on: ten_days_ago,
        }]
    );

    cleanup_lagging().await;
}
