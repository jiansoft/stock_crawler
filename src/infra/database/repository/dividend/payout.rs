//! 盈餘分配率：整批讀取股利列與各期 EPS，計算後以陣列參數一次寫回。

use anyhow::{Context, Result};
use rust_decimal::Decimal;
use sqlx::{Row, postgres::PgRow};

use crate::domain::dividend::payout::{PayoutDividend, PayoutRatios, PeriodEarnings};
use crate::infra::database;

/// 取得計算盈餘分配率所需的股利列與各期財報每股盈餘。
///
/// 涵蓋期間要看同一所屬年度的其他配息才決定得了，因此整批取回後在記憶體計算
/// （見 [`crate::domain::dividend::payout`]）；全市場約四萬多列股利、七萬列 EPS，
/// 在正式機（樹莓派）上也只佔數 MB。不排序：計算結果與順序無關。
///
/// 整批讀四萬多列在 pi 上約 1 秒，走慢查詢門檻 5 秒的批次連線池，避免每天誤報。
pub(super) async fn fetch_payout_ratio_inputs() -> Result<(Vec<PayoutDividend>, Vec<PeriodEarnings>)>
{
    // ETF 與債券只配息、沒有財報，分配率永遠算不出來，先整批擋掉。
    // 資料表裡有 year_of_dividend = 0 的殘留列，財報那邊也有 year = 0 的髒資料，一併排除。
    let dividend_sql = r#"
        SELECT d.serial, d.security_code, d.year, d.year_of_dividend, d.quarter,
               d.cash_dividend, d.stock_dividend, d."sum",
               d.payout_ratio_cash, d.payout_ratio_stock, d.payout_ratio,
               d.payout_eps, d.payout_period
        FROM dividend AS d
        WHERE d.year > 0
          AND EXISTS (
              SELECT 1
                FROM financial_statement AS fs
               WHERE fs.security_code = d.security_code
          )
    "#;
    let dividends = sqlx::query(dividend_sql)
        .try_map(|row: PgRow| {
            Ok(PayoutDividend {
                serial: row.try_get("serial")?,
                security_code: row.try_get("security_code")?,
                year: row.try_get("year")?,
                year_of_dividend: row.try_get("year_of_dividend")?,
                quarter: row.try_get("quarter")?,
                cash_dividend: row.try_get("cash_dividend")?,
                stock_dividend: row.try_get("stock_dividend")?,
                sum: row.try_get("sum")?,
                payout_ratio_cash: row.try_get("payout_ratio_cash")?,
                payout_ratio_stock: row.try_get("payout_ratio_stock")?,
                payout_ratio: row.try_get("payout_ratio")?,
                payout_eps: row.try_get("payout_eps")?,
                payout_period: row.try_get("payout_period")?,
            })
        })
        .fetch_all(database::get_batch_connection())
        .await
        .context("Failed to fetch dividends for payout ratios")?;

    // 只取股利所屬年度用得到的財報；季別限 Q1~Q4 與年報（空字串）。
    // financial_statement.year 是 bigint，要轉成 int 才能對上 dividend.year_of_dividend 的 i32。
    let earnings_sql = r#"
        SELECT fs.security_code, fs.year::int AS year, fs.quarter, fs.earnings_per_share
        FROM financial_statement AS fs
        WHERE fs.quarter IN ('', 'Q1', 'Q2', 'Q3', 'Q4')
          AND fs.year > 0
          AND EXISTS (
              SELECT 1
                FROM dividend AS d
               WHERE d.security_code = fs.security_code
                 AND d.year_of_dividend = fs.year
          )
    "#;
    let earnings = sqlx::query(earnings_sql)
        .try_map(|row: PgRow| {
            Ok(PeriodEarnings {
                security_code: row.try_get("security_code")?,
                year: row.try_get("year")?,
                quarter: row.try_get("quarter")?,
                earnings_per_share: row.try_get("earnings_per_share")?,
            })
        })
        .fetch_all(database::get_batch_connection())
        .await
        .context("Failed to fetch earnings for payout ratios")?;

    Ok((dividends, earnings))
}

/// 批次寫回計算好的盈餘分配率、分母 EPS 與涵蓋期間，回傳實際更新的列數。
///
/// 以陣列參數一次更新，避免上千列各發一次 UPDATE；資料量再大也只有一次來回。
pub(super) async fn update_payout_ratios(ratios: &[PayoutRatios]) -> Result<u64> {
    if ratios.is_empty() {
        return Ok(0);
    }

    let serials: Vec<i64> = ratios.iter().map(|ratio| ratio.serial).collect();
    let cash: Vec<Decimal> = ratios.iter().map(|ratio| ratio.payout_ratio_cash).collect();
    let stock: Vec<Decimal> = ratios
        .iter()
        .map(|ratio| ratio.payout_ratio_stock)
        .collect();
    let total: Vec<Decimal> = ratios.iter().map(|ratio| ratio.payout_ratio).collect();
    let eps: Vec<Decimal> = ratios.iter().map(|ratio| ratio.payout_eps).collect();
    let periods: Vec<String> = ratios
        .iter()
        .map(|ratio| ratio.payout_period.clone())
        .collect();

    let sql = r#"
        UPDATE dividend AS d
        SET payout_ratio_cash = u.payout_ratio_cash,
            payout_ratio_stock = u.payout_ratio_stock,
            payout_ratio = u.payout_ratio,
            payout_eps = u.payout_eps,
            payout_period = u.payout_period,
            updated_time = now()
        FROM UNNEST($1::bigint[], $2::numeric[], $3::numeric[], $4::numeric[], $5::numeric[], $6::varchar[])
            AS u(serial, payout_ratio_cash, payout_ratio_stock, payout_ratio, payout_eps, payout_period)
        WHERE d.serial = u.serial
    "#;

    let result = sqlx::query(sql)
        .bind(&serials)
        .bind(&cash)
        .bind(&stock)
        .bind(&total)
        .bind(&eps)
        .bind(&periods)
        .execute(database::get_connection())
        .await
        .context("Failed to update payout ratios")?;

    Ok(result.rows_affected())
}
