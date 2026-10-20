//! 股利識別（所屬年度＋期別）：不同年度同期別、更正年度與全年度事件的整合測試。

use super::*;

/// 主鍵含所屬年度的測試代號，與上面兩個代號分開清理，避免測試互相干擾。
const IDENTITY_SYMBOL: &str = "79977";

async fn cleanup_identity() -> Result<()> {
    sqlx::query("DELETE FROM dividend WHERE security_code = $1")
        .bind(IDENTITY_SYMBOL)
        .execute(database::get_connection())
        .await?;
    Ok(())
}

/// 讀回測試代號在測試發放年度的所有列，依期別與所屬年度排序。
async fn fetch_identity_rows() -> Result<Vec<Dividend>> {
    let sql = r#"
        SELECT
            serial, security_code, year, year_of_dividend, quarter,
            cash_dividend, stock_dividend, sum, "ex-dividend_date1", "ex-dividend_date2",
            payable_date1, payable_date2, created_time, updated_time,
            capital_reserve_cash_dividend, earnings_cash_dividend,
            capital_reserve_stock_dividend, earnings_stock_dividend,
            payout_ratio_cash, payout_ratio_stock, payout_ratio
        FROM dividend
        WHERE security_code = $1 AND year = $2
        ORDER BY quarter, year_of_dividend
    "#;
    sqlx::query(sql)
        .bind(IDENTITY_SYMBOL)
        .bind(TEST_PAYOUT_YEAR)
        .try_map(PgDividendRepository::row_to_entity)
        .fetch_all(database::get_connection())
        .await
        .context("測試資料讀取失敗")
}

/// 同一發放年度的兩次 H1（所屬年度不同，3008 大立光 2022 年的情況）要並存，
/// 年度合計把兩筆都算進去。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn same_period_of_different_fiscal_years_coexist() {
    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    cleanup_identity().await.expect("測試前清理失敗");

    for dividend in [
        build_dividend(
            IDENTITY_SYMBOL,
            TEST_PAYOUT_YEAR - 1,
            "H1",
            dec!(31.1561),
            "2020-01-12",
            "2020-02-10",
        ),
        build_dividend(
            IDENTITY_SYMBOL,
            TEST_PAYOUT_YEAR,
            "H1",
            dec!(39.5),
            "2020-08-18",
            "2020-09-10",
        ),
    ] {
        repo.save(&dividend).await.expect("寫入分期明細失敗");
    }
    repo.upsert_annual_total_dividend(IDENTITY_SYMBOL, TEST_PAYOUT_YEAR)
        .await
        .expect("年度合計失敗");

    let rows = fetch_identity_rows().await.expect("讀取失敗");
    cleanup_identity().await.expect("測試後清理失敗");

    let summary: Vec<(&str, i32, Decimal)> = rows
        .iter()
        .map(|row| (row.quarter.as_str(), row.year_of_dividend, row.sum))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("", TEST_PAYOUT_YEAR - 1, dec!(70.6561)),
            ("H1", TEST_PAYOUT_YEAR - 1, dec!(31.1561)),
            ("H1", TEST_PAYOUT_YEAR, dec!(39.5)),
        ]
    );
}

/// 同一次配息（除息日相同）只是所屬年度更正時，原地改寫那一列，不新增第二列，序號保留。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn corrected_year_of_dividend_updates_the_same_event() {
    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    cleanup_identity().await.expect("測試前清理失敗");

    repo.save(&build_dividend(
        IDENTITY_SYMBOL,
        TEST_PAYOUT_YEAR,
        "Q3",
        dec!(1.5),
        "2020-11-28",
        "-",
    ))
    .await
    .expect("首次寫入失敗");
    let before = fetch_identity_rows().await.expect("讀取失敗");

    repo.save(&build_dividend(
        IDENTITY_SYMBOL,
        TEST_PAYOUT_YEAR - 1,
        "Q3",
        dec!(1.5),
        "2020-11-28",
        "2020-12-20",
    ))
    .await
    .expect("更正寫入失敗");
    // 除息日不同就是另一次配息，照常新增。
    repo.save(&build_dividend(
        IDENTITY_SYMBOL,
        TEST_PAYOUT_YEAR,
        "Q3",
        dec!(2.0),
        "2020-02-26",
        "-",
    ))
    .await
    .expect("另一次配息寫入失敗");
    let after = fetch_identity_rows().await.expect("讀取失敗");
    cleanup_identity().await.expect("測試後清理失敗");

    assert_eq!(after.len(), 2);
    let corrected = after
        .iter()
        .find(|row| row.year_of_dividend == TEST_PAYOUT_YEAR - 1)
        .expect("更正後的列應存在");
    assert_eq!(corrected.serial, before[0].serial, "同一次配息要保留序號");
    assert_eq!(corrected.payable_date_cash, "2020-12-20");
    assert!(
        after
            .iter()
            .any(|row| row.year_of_dividend == TEST_PAYOUT_YEAR && row.sum == dec!(2.0))
    );
}

/// 年度層級列（quarter = ''）每個發放年度只有一列：所屬年度不同也是覆寫同一列。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn annual_level_row_stays_unique_per_payout_year() {
    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    cleanup_identity().await.expect("測試前清理失敗");

    repo.save(&build_dividend(
        IDENTITY_SYMBOL,
        TEST_PAYOUT_YEAR - 1,
        "",
        dec!(3.0),
        "2020-07-01",
        "-",
    ))
    .await
    .expect("首次寫入失敗");
    repo.save(&build_dividend(
        IDENTITY_SYMBOL,
        TEST_PAYOUT_YEAR,
        "",
        dec!(3.2),
        "2020-07-15",
        "-",
    ))
    .await
    .expect("覆寫失敗");
    let rows = fetch_identity_rows().await.expect("讀取失敗");
    cleanup_identity().await.expect("測試後清理失敗");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].year_of_dividend, TEST_PAYOUT_YEAR);
    assert_eq!(rows[0].sum, dec!(3.2));
}

/// 混合配息年度可以同時有兩個所屬年度的全年事件 A（5287 在 2022 年發 2020 年配股與 2021 年配息）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn full_year_events_of_two_fiscal_years_coexist() {
    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    cleanup_identity().await.expect("測試前清理失敗");

    for dividend in [
        build_dividend(
            IDENTITY_SYMBOL,
            TEST_PAYOUT_YEAR - 2,
            "A",
            dec!(1.8309),
            "-",
            "-",
        ),
        build_dividend(
            IDENTITY_SYMBOL,
            TEST_PAYOUT_YEAR - 1,
            "A",
            dec!(11.3),
            "2020-04-14",
            "-",
        ),
        build_dividend(
            IDENTITY_SYMBOL,
            TEST_PAYOUT_YEAR,
            "H1",
            dec!(4.5),
            "2020-11-03",
            "-",
        ),
    ] {
        repo.save(&dividend).await.expect("寫入失敗");
    }
    repo.upsert_annual_total_dividend(IDENTITY_SYMBOL, TEST_PAYOUT_YEAR)
        .await
        .expect("年度合計失敗");
    let rows = fetch_identity_rows().await.expect("讀取失敗");
    cleanup_identity().await.expect("測試後清理失敗");

    assert_eq!(rows.iter().filter(|row| row.quarter == "A").count(), 2);
    let total = rows
        .iter()
        .find(|row| row.quarter.is_empty())
        .expect("合計列");
    assert_eq!(total.sum, dec!(17.6309));
}
