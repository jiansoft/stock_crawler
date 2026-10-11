//! `PgFinancialHealthRepository` 的整合測試：以假代號種入三大報表，驗證讀取與整批取代。

use rust_decimal_macros::dec;

use super::*;

/// 假代號；A 是一般公司，B 只用來驗證整批取代。
const SYMBOL_A: &str = "79979HA";
const SYMBOL_B: &str = "79979HB";

async fn cleanup() {
    let pool = database::get_connection();
    for symbol in [SYMBOL_A, SYMBOL_B] {
        for sql in [
            "DELETE FROM income_statement WHERE stock_symbol = $1",
            "DELETE FROM balance_sheet WHERE stock_symbol = $1",
            "DELETE FROM cash_flow_statement WHERE stock_symbol = $1",
            "DELETE FROM financial_health WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(symbol).execute(pool).await;
        }
    }
}

async fn seed() {
    for sql in [
        // 2020 年報 Yahoo 與 BigGo 並存（應取 Yahoo）；2019 只有 Yahoo。
        r#"INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, gross_profit, net_income)
           VALUES ($1, 2020, 'annual', 'A', 'yahoo', 4600, 1900, 460),
                  ($1, 2020, 'annual', 'A', 'biggo', 4600, 1900, -9999),
                  ($1, 2019, 'annual', 'A', 'yahoo', 4000, 1500, 300),
                  ($1, 2020, 'single', 'Q3', 'yahoo', 1200, 500, 120),
                  ($1, 2020, 'single', 'Q4', 'yahoo', 1300, 550, 130),
                  ($1, 2020, 'single', 'Q4', 'biggo', 9999, 1, 1)"#,
        r#"INSERT INTO cash_flow_statement (stock_symbol, fiscal_year, period_type, quarter, source, operating_cash_flow, free_cash_flow)
           VALUES ($1, 2020, 'annual', 'A', 'yahoo', 900, 500),
                  ($1, 2020, 'single', 'Q3', 'yahoo', 200, 90),
                  ($1, 2020, 'single', 'Q4', 'yahoo', 300, 110)"#,
        // 2018 只有第 4 季資產負債表、沒有年度損益表，不應當成評分年度；
        // 2021-Q1 只有資產負債表，不應當成有損益表的季度。
        r#"INSERT INTO balance_sheet (stock_symbol, fiscal_year, period_type, quarter, source, total_assets, total_liabilities, current_assets, current_liabilities, non_current_liabilities, share_capital)
           VALUES ($1, 2018, 'single', 'Q4', 'yahoo', 45000, 18000, 14000, 8500, 9500, 1000),
                  ($1, 2019, 'single', 'Q4', 'yahoo', 48000, 19000, 15000, 9000, 10000, 1000),
                  ($1, 2020, 'single', 'Q3', 'yahoo', 50000, 20000, 18000, 9500, 10500, 1000),
                  ($1, 2020, 'single', 'Q4', 'yahoo', 52000, 21000, 20000, 10000, 11000, 1000),
                  ($1, 2021, 'single', 'Q1', 'yahoo', 53000, 21500, 21000, 10000, 11500, 1000)"#,
    ] {
        sqlx::query(sql)
            .bind(SYMBOL_A)
            .execute(database::get_connection())
            .await
            .unwrap_or_else(|error| panic!("種入測試資料失敗：{error}\n{sql}"));
    }
}

fn snapshot(symbol: &str, f_score: Option<u8>) -> FinancialHealthSnapshot {
    FinancialHealthSnapshot {
        stock_symbol: symbol.to_owned(),
        f_score_year: f_score.map(|_| 2020),
        f_score,
        f_score_evaluated: if f_score.is_some() { 9 } else { 0 },
        quarter_year: Some(2021),
        quarter: Some(1),
        gross_margin_percent: Some(dec!(42.3077)),
        debt_ratio_percent: None,
        free_cash_flow_ttm: Some(-200),
    }
}

/// 讀取：年報與季報依股票＋期間各一列、Yahoo 優先、只有資產負債表的期間標為沒有損益表；
/// 寫入：整批取代，上一輪的列不會殘留。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn financial_health_repository_reads_figures_and_replaces_table() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 financial_health_repository_reads_figures_and_replaces_table：無資料庫連接");
        return;
    }
    cleanup().await;
    seed().await;
    let repo = PgFinancialHealthRepository::new();

    let mut annual: Vec<_> = repo
        .fetch_annual_figures(2018)
        .await
        .expect("fetch_annual_figures")
        .into_iter()
        .filter(|(symbol, _, _)| symbol == SYMBOL_A)
        .collect();
    annual.sort_by_key(|(_, year, _)| *year);
    let years: Vec<_> = annual
        .iter()
        .map(|(_, year, a)| (*year, a.has_income))
        .collect();
    assert_eq!(years, [(2018, false), (2019, true), (2020, true)]);
    assert_eq!(annual[0].2.figures.total_assets, Some(45_000));
    let y2020 = annual[2].2.figures;
    assert_eq!(y2020.net_income, Some(460), "同期應取 Yahoo");
    assert_eq!(y2020.operating_cash_flow, Some(900));
    assert_eq!(y2020.total_assets, Some(52_000), "資產負債表取第 4 季季末");
    assert_eq!(y2020.non_current_liabilities, Some(11_000));
    assert_eq!(
        annual[1].2.figures.operating_cash_flow, None,
        "2019 沒有現金流量表"
    );

    let mut quarters: Vec<_> = repo
        .fetch_quarter_figures(2020)
        .await
        .expect("fetch_quarter_figures")
        .into_iter()
        .filter(|(symbol, _)| symbol == SYMBOL_A)
        .map(|(_, figures)| figures)
        .collect();
    quarters.sort_by_key(|q| (q.fiscal_year, q.quarter));
    let periods: Vec<_> = quarters
        .iter()
        .map(|q| (q.fiscal_year, q.quarter, q.has_income))
        .collect();
    assert_eq!(
        periods,
        [(2020, 3, true), (2020, 4, true), (2021, 1, false)]
    );
    let q4 = quarters[1];
    assert_eq!(
        (q4.revenue, q4.gross_profit),
        (Some(1_300), Some(550)),
        "同期應取 Yahoo"
    );
    assert_eq!(
        (q4.total_assets, q4.total_liabilities),
        (Some(52_000), Some(21_000))
    );
    assert_eq!(q4.free_cash_flow, Some(110));

    let written = repo
        .replace_financial_health(&[snapshot(SYMBOL_A, Some(8)), snapshot(SYMBOL_B, None)])
        .await
        .expect("replace_financial_health");
    assert_eq!(written, 2);
    // 讀回時全欄位與寫入的快照一致（只看假代號，測試庫可能有其他列）。
    let stored = |repo_rows: Vec<FinancialHealthSnapshot>| -> Vec<FinancialHealthSnapshot> {
        repo_rows
            .into_iter()
            .filter(|s| s.stock_symbol == SYMBOL_A || s.stock_symbol == SYMBOL_B)
            .collect()
    };
    assert_eq!(
        stored(
            repo.fetch_financial_health()
                .await
                .expect("fetch_financial_health")
        ),
        vec![snapshot(SYMBOL_A, Some(8)), snapshot(SYMBOL_B, None)]
    );

    repo.replace_financial_health(&[snapshot(SYMBOL_A, Some(7))])
        .await
        .expect("replace_financial_health");
    assert_eq!(
        stored(
            repo.fetch_financial_health()
                .await
                .expect("fetch_financial_health")
        ),
        vec![snapshot(SYMBOL_A, Some(7))]
    );

    cleanup().await;
}
