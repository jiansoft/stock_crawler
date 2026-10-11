//! nstock 補 ROE／ROA：純函式的單元測試，以及以假代號、假 nstock 走一輪的整合測試。

use std::sync::Mutex;

use anyhow::anyhow;
use rust_decimal_macros::dec;

use super::*;
use crate::{
    infra::cache::SHARE, infra::crawler::nstock::eps::EpsQuarter,
    infra::crawler::nstock::eps::EpsYear, infra::database,
};

/// 每股淨值、ROE、ROA 都給非零值的一季財報，測試再把要歸零的欄位改掉。
fn statement(symbol: &str, year: i64, quarter: &str) -> DomainFinancialStatement {
    DomainFinancialStatement {
        security_code: symbol.to_owned(),
        year,
        quarter: quarter.to_owned(),
        net_asset_value_per_share: dec!(30),
        earnings_per_share: dec!(2),
        return_on_equity: dec!(5),
        return_on_assets: dec!(3),
        ..DomainFinancialStatement::default()
    }
}

fn zero_roe_roa(mut fs: DomainFinancialStatement) -> DomainFinancialStatement {
    fs.return_on_equity = Decimal::ZERO;
    fs.return_on_assets = Decimal::ZERO;
    fs
}

fn eps_quarter(
    symbol: &str,
    year: i32,
    quarter: Quarter,
    roe: Decimal,
    roa: Decimal,
) -> EpsQuarter {
    EpsQuarter {
        stock_symbol: symbol.to_owned(),
        year,
        quarter,
        eps: dec!(1),
        roe,
        roa,
        cumulative_eps: dec!(1),
    }
}

/// 只有每股淨值為 0 的列 nstock 補不了，不列入待補；其餘依股票分組。
#[test]
fn pending_by_stock_skips_rows_with_only_zero_net_asset_value() {
    let mut nav_only = statement("2330", 2020, "Q1");
    nav_only.net_asset_value_per_share = Decimal::ZERO;
    let mut roa_only = statement("2330", 2021, "Q1");
    roa_only.return_on_assets = Decimal::ZERO;

    let pending = pending_by_stock(vec![
        nav_only,
        roa_only,
        zero_roe_roa(statement("2317", 2020, "Q1")),
        zero_roe_roa(statement("2330", 2019, "Q1")),
    ]);

    let years: Vec<(&str, Vec<i64>)> = pending
        .iter()
        .map(|(symbol, rows)| (symbol.as_str(), rows.iter().map(|fs| fs.year).collect()))
        .collect();
    assert_eq!(
        years,
        vec![("2317", vec![2020]), ("2330", vec![2021, 2019])]
    );
}

/// 兩者皆 0 等於沒補到；與現值相同不寫；任一個不同且有值才寫。
#[test]
fn should_write_requires_a_new_non_zero_value() {
    let zero = zero_roe_roa(statement("2330", 2020, "Q1"));
    assert!(!should_write(&zero, Decimal::ZERO, Decimal::ZERO));
    assert!(should_write(&zero, dec!(4.5), Decimal::ZERO));
    assert!(should_write(&zero, dec!(4.5), dec!(2.5)));

    let filled = statement("2330", 2020, "Q1");
    assert!(!should_write(&filled, dec!(5), dec!(3)));
    assert!(should_write(&filled, dec!(5), dec!(3.1)));
}

/// 季報用季度資料、年報用年度資料，key 與 `股票-年度-季度` 對得上。
#[test]
fn nstock_roe_roa_picks_quarters_or_years() {
    let eps = || Eps {
        quarters: vec![eps_quarter("2330", 2020, Quarter::Q1, dec!(4.5), dec!(2.5))],
        years: vec![EpsYear {
            stock_symbol: "2330".to_owned(),
            year: 2020,
            eps: dec!(20),
            roe: dec!(30),
            roa: dec!(20),
            operating_profit_margin: dec!(40),
            gross_profit: dec!(50),
        }],
    };

    let quarterly = nstock_roe_roa(eps(), Some(Quarter::Q1));
    assert_eq!(quarterly.get("2330-2020-Q1"), Some(&(dec!(4.5), dec!(2.5))));
    assert_eq!(quarterly.len(), 1);

    let annual = nstock_roe_roa(eps(), None);
    assert_eq!(annual.get("2330-2020-"), Some(&(dec!(30), dec!(20))));
    assert_eq!(annual.len(), 1);
}

#[test]
fn skip_cache_key_carries_symbol_and_scope() {
    assert_eq!(
        skip_cache_key("2330", Some(Quarter::Q2)),
        "nstock:roe_roa:2330:Q2"
    );
    assert_eq!(skip_cache_key("2330", None), "nstock:roe_roa:2330:annual");
}

/// 查過的年度涵蓋目前最新的待補列才略過；出現更新年度的待補列（新一季）照樣查。
#[test]
fn already_checked_lets_newer_years_through() {
    assert!(!already_checked(None, 2026));
    assert!(already_checked(Some(2026), 2026));
    assert!(already_checked(Some(2026), 2019));
    assert!(!already_checked(Some(2025), 2026));
}

#[test]
fn run_summary_lists_every_counter() {
    let run = RoeRoaRun {
        rows: 7,
        stocks: 5,
        skipped: 2,
        fetched: 3,
        failed: 1,
        updated: 4,
    };
    assert_eq!(
        run.to_string(),
        "rows=7 stocks=5 skipped=2 fetched=3 failed=1 updated=4"
    );
}

/// 假代號：A 有補得到與補不到的列，B 的 nstock 查詢失敗。
const SYMBOL_A: &str = "79979RA";
const SYMBOL_B: &str = "79979RB";

async fn cleanup() {
    let pool = database::get_connection();
    for symbol in [SYMBOL_A, SYMBOL_B] {
        for sql in [
            "DELETE FROM financial_statement WHERE security_code = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(symbol).execute(pool).await;
        }
        let _ = CLIENT
            .delete(&skip_cache_key(symbol, Some(Quarter::Q1)))
            .await;
    }
}

/// 假代號在資料庫裡 Q1 的待補列（ROE 或 ROA 為 0 或每股淨值為 0）。
async fn seeded_zero_rows(repo: &PgFinancialRepository) -> Vec<DomainFinancialStatement> {
    repo.fetch_roe_or_roa_equal_to_zero(None, Some(Quarter::Q1))
        .await
        .expect("fetch_roe_or_roa_equal_to_zero")
        .into_iter()
        .filter(|fs| fs.security_code == SYMBOL_A || fs.security_code == SYMBOL_B)
        .collect()
}

async fn roe_roa(year: i64) -> (Decimal, Decimal, chrono::DateTime<chrono::Local>) {
    sqlx::query_as(
        "SELECT return_on_equity, return_on_assets, updated_time FROM financial_statement WHERE security_code = $1 AND year = $2 AND quarter = 'Q1'",
    )
    .bind(SYMBOL_A)
    .bind(year)
    .fetch_one(database::get_connection())
    .await
    .expect("讀回假代號財報")
}

/// 只查 ROE／ROA 為 0 的股票、只寫有補到的列；查過的股票下一輪被跳過旗標擋住，
/// 查詢失敗的股票不寫旗標、下一輪再查。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fill_roe_roa_writes_found_values_and_skips_checked_stocks() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
        println!(
            "跳過 fill_roe_roa_writes_found_values_and_skips_checked_stocks：無資料庫或 Redis 連接"
        );
        return;
    }
    cleanup().await;
    for symbol in [SYMBOL_A, SYMBOL_B] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, 'ROE 補值測試', $1, 2, false)"#,
        )
        .bind(symbol)
        .execute(database::get_connection())
        .await
        .expect("種入股票主檔");
    }
    let repo = PgFinancialRepository::new();
    let mut nav_only = statement(SYMBOL_A, 2018, "Q1");
    nav_only.net_asset_value_per_share = Decimal::ZERO;
    repo.batch_save_financial_statements(&[
        zero_roe_roa(statement(SYMBOL_A, 2020, "Q1")),
        zero_roe_roa(statement(SYMBOL_A, 2019, "Q1")),
        nav_only,
        zero_roe_roa(statement(SYMBOL_B, 2020, "Q1")),
    ])
    .await
    .expect("種入假代號財報");
    let (_, _, seeded_at) = roe_roa(2020).await;

    let calls = Mutex::new(Vec::new());
    let fetch = |symbol: String| {
        calls.lock().expect("calls lock").push(symbol.clone());
        async move {
            if symbol == SYMBOL_B {
                return Err(anyhow!("nstock 測試失敗"));
            }
            Ok(Eps {
                quarters: vec![
                    eps_quarter(SYMBOL_A, 2020, Quarter::Q1, dec!(4.5), dec!(2.5)),
                    eps_quarter(SYMBOL_A, 2019, Quarter::Q1, Decimal::ZERO, Decimal::ZERO),
                    eps_quarter(SYMBOL_A, 2018, Quarter::Q1, dec!(9), dec!(9)),
                ],
                years: Vec::new(),
            })
        }
    };

    let run = fill_roe_roa(
        &repo,
        seeded_zero_rows(&repo).await,
        Some(Quarter::Q1),
        &fetch,
    )
    .await;
    assert_eq!(
        run,
        RoeRoaRun {
            rows: 3,
            stocks: 2,
            skipped: 0,
            fetched: 2,
            failed: 1,
            updated: 1,
        }
    );
    let (roe, roa, updated_at) = roe_roa(2020).await;
    assert_eq!((roe, roa), (dec!(4.5), dec!(2.5)));
    assert!(updated_at > seeded_at, "寫回時應更新 updated_time");
    let (roe, roa, _) = roe_roa(2019).await;
    assert_eq!(
        (roe, roa),
        (Decimal::ZERO, Decimal::ZERO),
        "nstock 也是 0 不應寫回"
    );
    let (roe, roa, _) = roe_roa(2018).await;
    assert_eq!(
        (roe, roa),
        (dec!(5), dec!(3)),
        "只有每股淨值為 0 的列不應被改動"
    );
    // A 記下查詢當時的最新年度；B 查詢失敗不寫旗標。
    assert_eq!(
        CLIENT
            .get_string(&skip_cache_key(SYMBOL_A, Some(Quarter::Q1)))
            .await
            .ok()
            .as_deref(),
        Some("2020")
    );
    assert!(
        CLIENT
            .get_string(&skip_cache_key(SYMBOL_B, Some(Quarter::Q1)))
            .await
            .is_err()
    );

    // 第二輪 A 只剩 2019 的待補列，被 2020 的旗標擋住；B 照樣再查。
    calls.lock().expect("calls lock").clear();
    let run = fill_roe_roa(
        &repo,
        seeded_zero_rows(&repo).await,
        Some(Quarter::Q1),
        &fetch,
    )
    .await;
    assert_eq!(
        (run.rows, run.skipped, run.fetched, run.updated),
        (2, 1, 1, 0)
    );
    assert_eq!(
        *calls.lock().expect("calls lock"),
        vec![SYMBOL_B.to_owned()]
    );

    cleanup().await;
}

/// 對 `.env` 的資料庫與 nstock 實際跑一次 Q3 補值（手動執行）。
#[tokio::test]
#[ignore]
async fn test_update_annual_roe_and_roa_for_zero_values() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    if let Err(why) = update_roe_and_roa_for_zero_values(Some(Quarter::Q3)).await {
        tracing::debug!("Failed to roe_and_roa because {:?}", why);
    }
}
