//! 股利的讀取查詢（依年度、代號、尚未公布日期與持有期間取出股利列）與配息日期更新。

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use sqlx::{Row, postgres::PgRow};

use super::PgDividendRepository;
use crate::domain::dividend::entity::Dividend;
use crate::infra::database;

/// 依證券代號查詢該證券的所有股利年度。
pub(super) async fn fetch_years_by_security_code(security_code: &str) -> Result<Vec<i32>> {
    let sql = r#"
        SELECT DISTINCT year 
        FROM dividend 
        WHERE security_code = $1 
        ORDER BY year DESC
    "#;
    let rows = sqlx::query(sql)
        .bind(security_code)
        .map(|row: PgRow| row.get::<i32, _>(0))
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch years by security code")?;
    Ok(rows)
}

/// 取得仍在上市櫃的採集候選，由呼叫端的三天快取限制重抓頻率。
/// 已有年配不能證明全年資料完整，公司可能在年中改為半年配。
pub(super) async fn fetch_dividend_refresh_candidates() -> Result<Vec<String>> {
    let sql = r#"
        SELECT stock_symbol
        FROM stocks
        WHERE "SuspendListing" = false
            AND stock_exchange_market_id IN (2, 4);
    "#;
    let stock_symbols: Vec<String> = sqlx::query(sql)
        .fetch_all(database::get_connection())
        .await?
        .into_iter()
        .map(|row: PgRow| row.get("stock_symbol"))
        .collect();
    Ok(stock_symbols)
}

/// 取得指定年度與多次配息相關的股利資料。
pub(super) async fn fetch_multiple_dividends_for_year(year: i32) -> Result<Vec<Dividend>> {
    let sql = r#"
        SELECT 
            serial, security_code, year, year_of_dividend, quarter,
            cash_dividend, stock_dividend, sum, "ex-dividend_date1", "ex-dividend_date2",
            payable_date1, payable_date2, created_time, updated_time,
            capital_reserve_cash_dividend, earnings_cash_dividend,
            capital_reserve_stock_dividend, earnings_stock_dividend,
            payout_ratio_cash, payout_ratio_stock, payout_ratio
        FROM dividend
        WHERE (year = $1 OR year_of_dividend = $1) AND quarter IN ('Q1','Q2','Q3','Q4','H1','H2');
    "#;
    let rows = sqlx::query(sql)
        .bind(year)
        .try_map(PgDividendRepository::row_to_entity)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch multiple dividends for year")?;
    Ok(rows)
}

/// 取得指定發放年度的所有股利資料。
pub(super) async fn fetch_by_years(years: &[i32]) -> Result<Vec<Dividend>> {
    // 年度清單為空時直接返回，避免送出 `year = ANY('{}')` 這種必然無結果的查詢。
    if years.is_empty() {
        return Ok(Vec::new());
    }

    let sql = r#"
        SELECT
            serial, security_code, year, year_of_dividend, quarter,
            cash_dividend, stock_dividend, sum, "ex-dividend_date1", "ex-dividend_date2",
            payable_date1, payable_date2, created_time, updated_time,
            capital_reserve_cash_dividend, earnings_cash_dividend,
            capital_reserve_stock_dividend, earnings_stock_dividend,
            payout_ratio_cash, payout_ratio_stock, payout_ratio
        FROM dividend
        WHERE year = ANY($1);
    "#;
    let rows = sqlx::query(sql)
        .bind(years)
        .try_map(PgDividendRepository::row_to_entity)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch dividends by years")?;
    Ok(rows)
}

/// 取得指定年度尚未有配息日或發放日的股息數據。
///
/// 年度合計列的日期永遠是 `'-'`，若不排除就會讓每一檔有合計列的股票每次排程都白打一次
/// Yahoo；而且 Yahoo 那邊根本沒有對應的期別（合計列的 `quarter` 是空字串，來源端的全年
/// 事件已改用 `A`），撈回來也補不到任何日期。
///
/// 已下市（`"SuspendListing"`）的股票也排除：下市前公告、來不及除息的股利永遠不會有日期，
/// Yahoo 個股頁也已移除（4987 科誠 2026-07 下市後每天 21:00 都對 Yahoo 404）。
pub(super) async fn fetch_unpublished_dividend_date_or_payable_date_for_specified_year(
    year: i32,
) -> Result<Vec<Dividend>> {
    let sql = r#"
        SELECT
            serial, security_code, year, year_of_dividend, quarter,
            cash_dividend, stock_dividend, sum, "ex-dividend_date1", "ex-dividend_date2",
            payable_date1, payable_date2, created_time, updated_time,
            capital_reserve_cash_dividend, earnings_cash_dividend,
            capital_reserve_stock_dividend, earnings_stock_dividend,
            payout_ratio_cash, payout_ratio_stock, payout_ratio
        FROM dividend AS d
        WHERE (year = $1 OR year_of_dividend = $1)
            AND (
                (
                    cash_dividend > 0
                    AND (
                        "ex-dividend_date1" IN ('-', '尚未公布')
                        OR payable_date1 IN ('-', '尚未公布')
                    )
                )
                OR
                (
                    stock_dividend > 0
                    AND (
                        "ex-dividend_date2" IN ('-', '尚未公布')
                        OR payable_date2 IN ('-', '尚未公布')
                    )
                )
            )
            AND (
                d.quarter <> ''
                OR NOT EXISTS (
                    SELECT 1
                    FROM dividend AS t
                    WHERE t.security_code = d.security_code
                        AND t.year = d.year
                        AND t.quarter <> ''
                )
            )
            AND NOT EXISTS (
                SELECT 1
                FROM stocks AS s
                WHERE s.stock_symbol = d.security_code
                    AND s."SuspendListing"
            );
    "#;
    let rows = sqlx::query(sql)
        .bind(year)
        .try_map(PgDividendRepository::row_to_entity)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch unpublished dividend/payable date for specified year")?;
    Ok(rows)
}

/// 依代號、年份及持有（建立）時間，查詢所有可能重疊的股利發放資料。
///
/// 只回傳真實的配息事件，年度合計列必須排除：`upsert_annual_total_dividend`
/// 會為同一發放年度多次配息的股票額外寫入一列 `quarter = ''` 的合計，
/// 那是明細的加總而非另一次配發。合計列入庫時日期填 `'-'`，本來就過不了日期條件，
/// 但它若是由既有的年度配息列原地 upsert 而成（例如 2072 先有 2025 年配、
/// 隔年才改半年配），舊日期會留在該列上而讓合計被當成第三次配息重複計算。
///
/// 只有年配的股票其唯一明細列 `quarter` 同樣是空字串，所以不能直接用
/// `quarter <> ''` 過濾，必須確認該發放年度另有分期明細才認定為合計列。
pub(super) async fn fetch_dividends_summary_by_date(
    security_code: &str,
    year: i32,
    created_time: DateTime<Local>,
) -> Result<Vec<Dividend>> {
    let sql = r#"
        SELECT
            serial, security_code, year, year_of_dividend, quarter,
            cash_dividend, stock_dividend, sum, "ex-dividend_date1", "ex-dividend_date2",
            payable_date1, payable_date2, created_time, updated_time,
            capital_reserve_cash_dividend, earnings_cash_dividend,
            capital_reserve_stock_dividend, earnings_stock_dividend,
            payout_ratio_cash, payout_ratio_stock, payout_ratio
        FROM dividend AS d
        WHERE security_code = $1
            AND year = $2
            AND ("ex-dividend_date1" <= $3)
            AND ("ex-dividend_date1" >= $4 OR "ex-dividend_date2" >= $4)
            AND (
                d.quarter <> ''
                OR NOT EXISTS (
                    SELECT 1
                    FROM dividend AS t
                    WHERE t.security_code = d.security_code
                        AND t.year = d.year
                        AND t.quarter <> ''
                )
            )
    "#;

    let rows = sqlx::query(sql)
        .bind(security_code)
        .bind(year)
        .bind(Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
        .bind(created_time.format("%Y-%m-%d %H:%M:%S").to_string())
        .try_map(PgDividendRepository::row_to_entity)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch dividends summary by date")?;
    Ok(rows)
}

/// 更新股利發放日期相關資訊（除息日、除權日、發放日）。
pub(super) async fn update_dividend_date(dividend: &Dividend) -> Result<()> {
    let mut dividend = dividend.clone();
    dividend.clear_dates_of_zero_component();
    let sql = r#"
        UPDATE dividend
        SET
            "ex-dividend_date1" = $2,
            "ex-dividend_date2" = $3,
            payable_date1 = $4,
            payable_date2 = $5,
            updated_time = NOW()
        WHERE serial = $1;
    "#;
    sqlx::query(sql)
        .bind(dividend.serial)
        .bind(&dividend.ex_dividend_date_cash)
        .bind(&dividend.ex_dividend_date_stock)
        .bind(&dividend.payable_date_cash)
        .bind(&dividend.payable_date_stock)
        .execute(database::get_connection())
        .await
        .context("Failed to update dividend date in PgDividendRepository")?;
    Ok(())
}
