//! [`PgDividendRepository`] 的整合測試（需要 PostgreSQL；固定使用歷史年度與假代號 79977～79979）。
//!
//! 子模組：[`payout`]（盈餘分配率）、[`identity`]（股利識別）；這裡放共用的假資料、清理與年度合計列的測試。

mod identity;
mod payout;

use super::*;
use crate::infra::database;
use anyhow::Context;
use chrono::TimeZone;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// 混合配息年度的測試代號：同一發放年度同時有半年配與全年事件，因此會有年度合計列。
pub(super) const MIXED_YEAR_SYMBOL: &str = "79979";
/// 純年配的測試代號：唯一那筆明細的 `quarter` 也是空字串，不可以被當成合計列濾掉。
pub(super) const ANNUAL_ONLY_SYMBOL: &str = "79978";
/// 測試資料的發放年度；固定用歷史年度，避免結果隨系統時間變動。
pub(super) const TEST_PAYOUT_YEAR: i32 = 2020;

/// 組出一筆測試用股利；日期以字串傳入，方便直接鋪出「合計列殘留日期」的情境。
pub(super) fn build_dividend(
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
pub(super) async fn cleanup() -> Result<()> {
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
pub(super) async fn fetch_row(security_code: &str, quarter: &str) -> Result<Dividend> {
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
