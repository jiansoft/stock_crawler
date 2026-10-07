//! [`PgDividendRepository`] 的整合測試（需要 PostgreSQL；固定使用歷史年度與假代號 79977～79979）。

use chrono::TimeZone;
use rust_decimal_macros::dec;

use super::*;

/// 混合配息年度的測試代號：同一發放年度同時有半年配與全年事件，因此會有年度合計列。
const MIXED_YEAR_SYMBOL: &str = "79979";
/// 純年配的測試代號：唯一那筆明細的 `quarter` 也是空字串，不可以被當成合計列濾掉。
const ANNUAL_ONLY_SYMBOL: &str = "79978";
/// 測試資料的發放年度；固定用歷史年度，避免結果隨系統時間變動。
const TEST_PAYOUT_YEAR: i32 = 2020;

/// 組出一筆測試用股利；日期以字串傳入，方便直接鋪出「合計列殘留日期」的情境。
fn build_dividend(
    security_code: &str,
    year_of_dividend: i32,
    quarter: &str,
    cash_dividend: Decimal,
    ex_dividend_date_cash: &str,
    payable_date_cash: &str,
) -> Dividend {
    Dividend {
        serial: 0,
        year: TEST_PAYOUT_YEAR,
        year_of_dividend,
        quarter: quarter.to_string(),
        security_code: security_code.to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend,
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: Decimal::ZERO,
        sum: cash_dividend,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: ex_dividend_date_cash.to_string(),
        ex_dividend_date_stock: "-".to_string(),
        payable_date_cash: payable_date_cash.to_string(),
        payable_date_stock: "-".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    }
}

/// 清掉測試代號留下的資料列；測試開頭與結尾都要呼叫，避免上一輪殘留影響斷言。
async fn cleanup() -> Result<()> {
    sqlx::query("DELETE FROM dividend WHERE security_code = ANY($1)")
        .bind(vec![
            MIXED_YEAR_SYMBOL.to_string(),
            ANNUAL_ONLY_SYMBOL.to_string(),
        ])
        .execute(database::get_connection())
        .await?;
    Ok(())
}

/// 直接讀回單一資料列，用來驗證寫入後的日期欄位。
async fn fetch_row(security_code: &str, quarter: &str) -> Result<Dividend> {
    let sql = r#"
        SELECT
            serial, security_code, year, year_of_dividend, quarter,
            cash_dividend, stock_dividend, sum, "ex-dividend_date1", "ex-dividend_date2",
            payable_date1, payable_date2, created_time, updated_time,
            capital_reserve_cash_dividend, earnings_cash_dividend,
            capital_reserve_stock_dividend, earnings_stock_dividend,
            payout_ratio_cash, payout_ratio_stock, payout_ratio
        FROM dividend
        WHERE security_code = $1 AND year = $2 AND quarter = $3
    "#;

    sqlx::query(sql)
        .bind(security_code)
        .bind(TEST_PAYOUT_YEAR)
        .bind(quarter)
        .try_map(PgDividendRepository::row_to_entity)
        .fetch_one(database::get_connection())
        .await
        .context("測試資料讀取失敗")
}

/// 合計列不能被當成一次配息，但純年配的單列必須照常回傳。
///
/// 對應 2072 世紀風電：2026 年發放 2025 年配 7.0833 元與 2026H1 的 6 元，
/// 合計列 13.0833 元殘留了年配的除息日，於是被算成第三次配息。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn annual_total_row_is_excluded_but_year_only_dividend_is_kept() {
    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    cleanup().await.expect("測試前清理失敗");

    // 混合年度：半年配 + 全年事件 + 帶著殘留日期的年度合計列。
    for dividend in [
        build_dividend(
            MIXED_YEAR_SYMBOL,
            TEST_PAYOUT_YEAR,
            "H1",
            dec!(6),
            "2020-08-28",
            "2020-09-29",
        ),
        build_dividend(
            MIXED_YEAR_SYMBOL,
            TEST_PAYOUT_YEAR - 1,
            "A",
            dec!(7.0833),
            "2020-04-10",
            "2020-04-30",
        ),
        build_dividend(
            MIXED_YEAR_SYMBOL,
            TEST_PAYOUT_YEAR - 1,
            "",
            dec!(13.0833),
            "2020-04-10",
            "2020-04-30",
        ),
    ] {
        repo.save(&dividend).await.expect("測試資料寫入失敗");
    }

    // 純年配：唯一一列的 quarter 同樣是空字串，但它是真實配息。
    repo.save(&build_dividend(
        ANNUAL_ONLY_SYMBOL,
        TEST_PAYOUT_YEAR - 1,
        "",
        dec!(5),
        "2020-04-10",
        "2020-04-30",
    ))
    .await
    .expect("測試資料寫入失敗");

    let holding_date = Local
        .with_ymd_and_hms(TEST_PAYOUT_YEAR - 1, 1, 1, 0, 0, 0)
        .single()
        .expect("測試持有日轉換失敗");

    let mixed = repo
        .fetch_dividends_summary_by_date(MIXED_YEAR_SYMBOL, TEST_PAYOUT_YEAR, holding_date)
        .await
        .expect("混合配息年度查詢失敗");
    let mut mixed_quarters: Vec<String> = mixed.iter().map(|item| item.quarter.clone()).collect();
    mixed_quarters.sort();
    assert_eq!(
        mixed_quarters,
        vec!["A".to_string(), "H1".to_string()],
        "年度合計列不該被當成一次配息"
    );
    assert_eq!(
        mixed.iter().map(|item| item.cash_dividend).sum::<Decimal>(),
        dec!(13.0833),
        "配息金額應等於兩次實際配發的加總"
    );

    let annual_only = repo
        .fetch_dividends_summary_by_date(ANNUAL_ONLY_SYMBOL, TEST_PAYOUT_YEAR, holding_date)
        .await
        .expect("純年配查詢失敗");
    assert_eq!(annual_only.len(), 1, "純年配的唯一明細不可被濾掉");
    assert_eq!(annual_only[0].quarter, "", "純年配的期別本來就是空字串");

    cleanup().await.expect("測試後清理失敗");
}

/// 重算合計列時必須把殘留的日期清回 `'-'`。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn upsert_annual_total_dividend_resets_stale_dates() {
    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    cleanup().await.expect("測試前清理失敗");

    for dividend in [
        build_dividend(
            MIXED_YEAR_SYMBOL,
            TEST_PAYOUT_YEAR,
            "H1",
            dec!(6),
            "2020-08-28",
            "2020-09-29",
        ),
        build_dividend(
            MIXED_YEAR_SYMBOL,
            TEST_PAYOUT_YEAR - 1,
            "A",
            dec!(7.0833),
            "2020-04-10",
            "2020-04-30",
        ),
        // 由年配明細原地轉生的合計列：金額已是合計，日期卻還留著原本那次配發的。
        build_dividend(
            MIXED_YEAR_SYMBOL,
            TEST_PAYOUT_YEAR - 1,
            "",
            dec!(13.0833),
            "2020-04-10",
            "2020-04-30",
        ),
    ] {
        repo.save(&dividend).await.expect("測試資料寫入失敗");
    }

    repo.upsert_annual_total_dividend(MIXED_YEAR_SYMBOL, TEST_PAYOUT_YEAR)
        .await
        .expect("年度合計重算失敗");

    let total = fetch_row(MIXED_YEAR_SYMBOL, "")
        .await
        .expect("年度合計列讀取失敗");
    assert_eq!(total.sum, dec!(13.0833), "合計應等於兩次配發的加總");
    assert_eq!(total.ex_dividend_date_cash, "-", "合計列不該有除息日");
    assert_eq!(total.ex_dividend_date_stock, "-", "合計列不該有除權日");
    assert_eq!(total.payable_date_cash, "-", "合計列不該有現金股利發放日");
    assert_eq!(total.payable_date_stock, "-", "合計列不該有股票股利發放日");

    // 全年事件（quarter = 'A'）不受重算影響，日期必須原封不動。
    let full_year = fetch_row(MIXED_YEAR_SYMBOL, "A")
        .await
        .expect("全年事件讀取失敗");
    assert_eq!(full_year.ex_dividend_date_cash, "2020-04-10");
    assert_eq!(full_year.payable_date_cash, "2020-04-30");

    cleanup().await.expect("測試後清理失敗");
}

/// 清掉測試代號寫進 financial_statement 的財報。
async fn cleanup_earnings() -> Result<()> {
    sqlx::query("DELETE FROM financial_statement WHERE security_code = $1")
        .bind(MIXED_YEAR_SYMBOL)
        .execute(database::get_connection())
        .await?;
    Ok(())
}

/// 盈餘分配率從讀取、計算到寫回走一遍，確認欄位型別與 SQL 都對得上
/// （`financial_statement.year` 是 bigint，曾因直接解碼成 i32 讓排程失敗）。
///
/// 仿 4735 豪展的季配息：同一發放年度有 2019Q3、2019Q4、2020Q1 三筆，
/// 年度合計的分母是三者涵蓋期間 EPS 的合計（2019Q1~Q3 1.7 + 2019Q4 0.74 + 2020Q1 1.19）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn payout_ratios_round_trip_with_covered_period() {
    use crate::domain::dividend::payout::calculate_payout_ratios;

    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    cleanup().await.expect("測試前清理失敗");
    cleanup_earnings().await.expect("測試前清理財報失敗");

    for (year, quarter, eps) in [
        (2019_i64, "Q1", dec!(0.29)),
        (2019, "Q2", dec!(0.51)),
        (2019, "Q3", dec!(0.9)),
        (2019, "Q4", dec!(0.74)),
        (2020, "Q1", dec!(1.19)),
    ] {
        sqlx::query(
            "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share) VALUES ($1, $2, $3, $4)",
        )
        .bind(MIXED_YEAR_SYMBOL)
        .bind(year)
        .bind(quarter)
        .bind(eps)
        .execute(database::get_connection())
        .await
        .expect("測試財報寫入失敗");
    }
    for dividend in [
        build_dividend(
            MIXED_YEAR_SYMBOL,
            2019,
            "Q3",
            dec!(0.7),
            "2019-12-02",
            "2020-01-10",
        ),
        build_dividend(
            MIXED_YEAR_SYMBOL,
            2019,
            "Q4",
            dec!(0.5),
            "2020-04-10",
            "2020-04-29",
        ),
        build_dividend(
            MIXED_YEAR_SYMBOL,
            2020,
            "Q1",
            dec!(1),
            "2020-06-15",
            "2020-07-08",
        ),
    ] {
        repo.save(&dividend).await.expect("測試資料寫入失敗");
    }
    repo.upsert_annual_total_dividend(MIXED_YEAR_SYMBOL, TEST_PAYOUT_YEAR)
        .await
        .expect("年度合計重算失敗");

    let (dividends, earnings) = repo
        .fetch_payout_ratio_inputs()
        .await
        .expect("讀取分配率計算資料失敗");
    let ours: Vec<_> = dividends
        .into_iter()
        .filter(|row| row.security_code == MIXED_YEAR_SYMBOL)
        .collect();
    let ratios = calculate_payout_ratios(&ours, &earnings);
    assert_eq!(
        ratios.len(),
        4,
        "三筆配息加一列年度合計都要算出來：{ratios:?}"
    );
    repo.update_payout_ratios(&ratios)
        .await
        .expect("寫回分配率失敗");

    let (payout_eps, payout_period, payout_ratio): (Option<Decimal>, Option<String>, Decimal) =
        sqlx::query_as(
            "SELECT payout_eps, payout_period, payout_ratio FROM dividend WHERE security_code = $1 AND year = $2 AND quarter = ''",
        )
        .bind(MIXED_YEAR_SYMBOL)
        .bind(TEST_PAYOUT_YEAR)
        .fetch_one(database::get_connection())
        .await
        .expect("年度合計列讀取失敗");
    assert_eq!(payout_eps, Some(dec!(3.63)));
    assert_eq!(payout_period.as_deref(), Some("2019Q1~2020Q1"));
    assert_eq!(payout_ratio, dec!(60.6061));

    // 再算一次不該有任何列需要更新。
    let (dividends, earnings) = repo
        .fetch_payout_ratio_inputs()
        .await
        .expect("第二次讀取失敗");
    let ours: Vec<_> = dividends
        .into_iter()
        .filter(|row| row.security_code == MIXED_YEAR_SYMBOL)
        .collect();
    assert!(calculate_payout_ratios(&ours, &earnings).is_empty());

    cleanup().await.expect("測試後清理失敗");
    cleanup_earnings().await.expect("測試後清理財報失敗");
}

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

/// 未公布日期的查詢要排除已下市股票：下市前公告的股利永遠不會有日期（4987 科誠）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn unpublished_dates_skip_delisted_stocks() {
    dotenvy::dotenv().ok();
    const DELISTED: &str = "79985";
    const LISTED: &str = "79986";
    let pool = database::get_connection();
    let symbols = vec![DELISTED.to_string(), LISTED.to_string()];
    let cleanup = || async {
        sqlx::query("DELETE FROM dividend WHERE security_code = ANY($1)")
            .bind(&symbols)
            .execute(pool)
            .await
            .expect("清除股利");
        sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
            .bind(&symbols)
            .execute(pool)
            .await
            .expect("清除股票主檔");
    };
    cleanup().await;

    let repo = PgDividendRepository::new();
    for (symbol, suspend_listing) in [(DELISTED, true), (LISTED, false)] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, "SuspendListing")
               VALUES ($1, $1, $1, $2)"#,
        )
        .bind(symbol)
        .bind(suspend_listing)
        .execute(pool)
        .await
        .expect("插入股票主檔");
        repo.save(&build_dividend(
            symbol,
            TEST_PAYOUT_YEAR,
            "Q3",
            dec!(1.5),
            "尚未公布",
            "尚未公布",
        ))
        .await
        .expect("測試資料寫入失敗");
    }

    let found: Vec<String> = repo
        .fetch_unpublished_dividend_date_or_payable_date_for_specified_year(TEST_PAYOUT_YEAR)
        .await
        .expect("查詢未公布日期失敗")
        .into_iter()
        .map(|dividend| dividend.security_code)
        .filter(|symbol| symbols.contains(symbol))
        .collect();
    assert_eq!(found, vec![LISTED.to_string()], "已下市股票不該再補日期");

    cleanup().await;
}
