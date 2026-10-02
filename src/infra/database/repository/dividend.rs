use crate::domain::dividend::entity::{
    Dividend, StockDividendInfo as DomainStockDividendInfo,
    StockDividendPayableDateInfo as DomainStockDividendPayableDateInfo,
};
use crate::domain::dividend::payout::{PayoutDividend, PayoutRatios, PeriodEarnings};
use crate::domain::dividend::repository::DividendRepository;
use crate::infra::database;
use crate::infra::database::table::dividend::extension::stock_dividend_info::{
    self, StockDividendInfo as TableStockDividendInfo,
};
use crate::infra::database::table::dividend::extension::stock_dividend_payable_date_info::StockDividendPayableDateInfo as TableStockDividendPayableDateInfo;
use crate::infra::database::table::dividend::mutation;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::{Row, postgres::PgRow};

impl From<TableStockDividendInfo> for DomainStockDividendInfo {
    fn from(table: TableStockDividendInfo) -> Self {
        DomainStockDividendInfo {
            stock_symbol: table.stock_symbol,
            name: table.name,
            stock_industry_id: table.stock_industry_id,
            cash_dividend: table.cash_dividend,
            stock_dividend: table.stock_dividend,
            sum: table.sum,
            closing_price: table.closing_price,
            dividend_yield: table.dividend_yield,
            cash_dividend_yield: table.cash_dividend_yield,
            is_cash_ex_dividend_on_date: table.is_cash_ex_dividend_on_date,
            is_stock_ex_dividend_on_date: table.is_stock_ex_dividend_on_date,
        }
    }
}

impl From<TableStockDividendPayableDateInfo> for DomainStockDividendPayableDateInfo {
    fn from(table: TableStockDividendPayableDateInfo) -> Self {
        DomainStockDividendPayableDateInfo {
            stock_symbol: table.stock_symbol,
            name: table.name,
            cash_dividend: table.cash_dividend,
            stock_dividend: table.stock_dividend,
            sum: table.sum,
            payable_date1: table.payable_date1,
            payable_date2: table.payable_date2,
            ex_dividend_date1: table.ex_dividend_date1,
            ex_dividend_date2: table.ex_dividend_date2,
        }
    }
}

/// 基於 PostgreSQL 的股利倉儲實現 (PgDividendRepository)。
pub struct PgDividendRepository;

impl PgDividendRepository {
    /// 建立新的 PgDividendRepository 實例。
    pub fn new() -> Self {
        PgDividendRepository
    }

    /// 將資料庫的 `PgRow` 轉換成領域實體 `Dividend`。
    fn row_to_entity(row: PgRow) -> Result<Dividend, sqlx::Error> {
        Ok(Dividend {
            serial: row.try_get("serial")?,
            security_code: row.try_get("security_code")?,
            year: row.try_get("year")?,
            year_of_dividend: row.try_get("year_of_dividend")?,
            quarter: row.try_get("quarter")?,
            cash_dividend: row.try_get("cash_dividend")?,
            stock_dividend: row.try_get("stock_dividend")?,
            sum: row.try_get("sum")?,
            ex_dividend_date_cash: row.try_get("ex-dividend_date1")?,
            ex_dividend_date_stock: row.try_get("ex-dividend_date2")?,
            payable_date_cash: row.try_get("payable_date1")?,
            payable_date_stock: row.try_get("payable_date2")?,
            created_time: row.try_get("created_time")?,
            updated_time: row.try_get("updated_time")?,
            capital_reserve_cash_dividend: row.try_get("capital_reserve_cash_dividend")?,
            earnings_cash_dividend: row.try_get("earnings_cash_dividend")?,
            capital_reserve_stock_dividend: row.try_get("capital_reserve_stock_dividend")?,
            earnings_stock_dividend: row.try_get("earnings_stock_dividend")?,
            payout_ratio_cash: row.try_get("payout_ratio_cash")?,
            payout_ratio_stock: row.try_get("payout_ratio_stock")?,
            payout_ratio: row.try_get("payout_ratio")?,
        })
    }
}

/// 組出 [`PgDividendRepository::save`] 的 upsert SQL；`$target` 為衝突目標。
macro_rules! save_sql {
    ($target:expr) => {
        concat!(
            r#"
            INSERT INTO dividend (
                security_code, "year", year_of_dividend, quarter,
                cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2",
                payable_date1, payable_date2, created_time, updated_time, capital_reserve_cash_dividend,
                earnings_cash_dividend, capital_reserve_stock_dividend, earnings_stock_dividend,
                payout_ratio_cash, payout_ratio_stock, payout_ratio)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
            ON CONFLICT "#,
            $target,
            r#" DO UPDATE SET
                year_of_dividend = EXCLUDED.year_of_dividend,
                cash_dividend = EXCLUDED.cash_dividend,
                stock_dividend = EXCLUDED.stock_dividend,
                "sum" = EXCLUDED."sum",
                "ex-dividend_date1" = EXCLUDED."ex-dividend_date1",
                "ex-dividend_date2" = EXCLUDED."ex-dividend_date2",
                payable_date1 = EXCLUDED.payable_date1,
                payable_date2 = EXCLUDED.payable_date2,
                updated_time = EXCLUDED.updated_time,
                capital_reserve_cash_dividend = EXCLUDED.capital_reserve_cash_dividend,
                earnings_cash_dividend = EXCLUDED.earnings_cash_dividend,
                capital_reserve_stock_dividend = EXCLUDED.capital_reserve_stock_dividend,
                earnings_stock_dividend = EXCLUDED.earnings_stock_dividend,
                -- 盈餘分配率只有 Goodinfo 提供，Yahoo 一律送 0。
                -- 直接覆蓋會讓每次股利採集都洗掉已回補的分配率，所以來源是 0 時保留既有值。
                payout_ratio_cash = CASE WHEN EXCLUDED.payout_ratio_cash = 0
                    THEN dividend.payout_ratio_cash ELSE EXCLUDED.payout_ratio_cash END,
                payout_ratio_stock = CASE WHEN EXCLUDED.payout_ratio_stock = 0
                    THEN dividend.payout_ratio_stock ELSE EXCLUDED.payout_ratio_stock END,
                payout_ratio = CASE WHEN EXCLUDED.payout_ratio = 0
                    THEN dividend.payout_ratio ELSE EXCLUDED.payout_ratio END;
        "#
        )
    };
}

/// 年度層級列的 upsert：每個發放年度一列。
const SAVE_ANNUAL_LEVEL_SQL: &str = save_sql!(mutation::annual_level_conflict_target!());
/// 分期明細的 upsert：以主鍵為衝突目標。
const SAVE_DETAIL_SQL: &str = save_sql!(mutation::detail_conflict_target!());

/// 同一發放年度、同期別、除權息日相同但所屬年度不同的列，視為同一次配息，改成新的所屬年度。
///
/// 參數：`$1` 代號、`$2` 發放年度、`$3` 新的所屬年度、`$4` 期別、`$5` 除息日、`$6` 除權日。
/// 新所屬年度若已有自己的一列就不動（那是另一次配息）。
const RELABEL_SAME_EVENT_SQL: &str = r#"
    UPDATE dividend SET year_of_dividend = $3
    WHERE security_code = $1 AND year = $2 AND quarter = $4 AND year_of_dividend <> $3
      AND (("ex-dividend_date1" = $5 AND $5 ~ '^\d{4}-\d{2}-\d{2}$')
        OR ("ex-dividend_date2" = $6 AND $6 ~ '^\d{4}-\d{2}-\d{2}$'))
      AND NOT EXISTS (
          SELECT 1 FROM dividend
          WHERE security_code = $1 AND year = $2 AND year_of_dividend = $3 AND quarter = $4
      )
"#;

impl Default for PgDividendRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DividendRepository for PgDividendRepository {
    /// 依證券代號查詢該證券的所有股利年度。
    async fn fetch_years_by_security_code(&self, security_code: &str) -> Result<Vec<i32>> {
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
    async fn fetch_dividend_refresh_candidates(&self) -> Result<Vec<String>> {
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
    async fn fetch_multiple_dividends_for_year(&self, year: i32) -> Result<Vec<Dividend>> {
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
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch multiple dividends for year")?;
        Ok(rows)
    }

    /// 取得指定發放年度的所有股利資料。
    async fn fetch_by_years(&self, years: &[i32]) -> Result<Vec<Dividend>> {
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
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch dividends by years")?;
        Ok(rows)
    }

    /// 合併並更新指定股票在指定發放年度的年度股利合計。
    ///
    /// 合計列是明細的加總而非一次配發，日期一律為 `'-'`。衝突更新時必須連日期一起覆寫：
    /// 該列可能是由既有的年度配息列原地轉生（股票從年配改成半年配時，原本 `quarter = ''`
    /// 的明細列會與新的合計列撞上同一個年度層級唯一鍵），留著舊日期會讓合計被下游當成真實的配息事件。
    async fn upsert_annual_total_dividend(&self, security_code: &str, year: i32) -> Result<()> {
        let sql = r#"
            INSERT INTO dividend(security_code,
                   year,
                   year_of_dividend,
                   quarter,
                   cash_dividend,
                   stock_dividend,
                   sum,
                   "ex-dividend_date1",
                   "ex-dividend_date2",
                   payable_date1,
                   payable_date2,
                   created_time,
                   updated_time,
                   capital_reserve_cash_dividend,
                   earnings_cash_dividend,
                   capital_reserve_stock_dividend,
                   earnings_stock_dividend,
                   payout_ratio_cash,
                   payout_ratio_stock,
                   payout_ratio)
            SELECT security_code,
                   $1,
                   $2,
                   '',
                   sum(cash_dividend) as cash_dividend,
                   sum(stock_dividend) as stock_dividend,
                   sum(sum) as sum,
                   '-',
                   '-',
                   '-',
                   '-',
                   now(),
                   now(),
                   0,
                   0,
                   0,
                   0,
                   0,
                   0,
                   0
                   from dividend
            where security_code = $3 and year = $4 and quarter != ''
            group by security_code
            order by security_code
            ON CONFLICT (security_code, "year") WHERE quarter = '' DO UPDATE SET
                year_of_dividend = EXCLUDED.year_of_dividend,
                cash_dividend = EXCLUDED.cash_dividend,
                stock_dividend = EXCLUDED.stock_dividend,
                sum = EXCLUDED.sum,
                "ex-dividend_date1" = EXCLUDED."ex-dividend_date1",
                "ex-dividend_date2" = EXCLUDED."ex-dividend_date2",
                payable_date1 = EXCLUDED.payable_date1,
                payable_date2 = EXCLUDED.payable_date2,
                updated_time = EXCLUDED.updated_time;
        "#;
        sqlx::query(sql)
            .bind(year)
            .bind(year - 1)
            .bind(security_code)
            .bind(year)
            .execute(database::get_connection())
            .await
            .context("Failed to upsert annual total dividend")?;
        Ok(())
    }

    /// 找出帶有配息日期的年度合計列，回傳 (證券代號, 發放年度)。
    async fn fetch_stale_annual_total_dividends(&self) -> Result<Vec<(String, i32)>> {
        let sql = r#"
            SELECT d.security_code, d.year
            FROM dividend AS d
            WHERE d.quarter = ''
                AND (
                    d."ex-dividend_date1" <> '-'
                    OR d."ex-dividend_date2" <> '-'
                    OR d.payable_date1 <> '-'
                    OR d.payable_date2 <> '-'
                )
                AND EXISTS (
                    SELECT 1
                    FROM dividend AS t
                    WHERE t.security_code = d.security_code
                        AND t.year = d.year
                        AND t.quarter <> ''
                )
            ORDER BY d.security_code, d.year;
        "#;

        let rows = sqlx::query_as(sql)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch stale annual total dividends")?;
        Ok(rows)
    }

    /// 取得指定年度尚未有配息日或發放日的股息數據。
    ///
    /// 年度合計列的日期永遠是 `'-'`，若不排除就會讓每一檔有合計列的股票每次排程都白打一次
    /// Yahoo；而且 Yahoo 那邊根本沒有對應的期別（合計列的 `quarter` 是空字串，來源端的全年
    /// 事件已改用 `A`），撈回來也補不到任何日期。
    async fn fetch_unpublished_dividend_date_or_payable_date_for_specified_year(
        &self,
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
                );
        "#;
        let rows = sqlx::query(sql)
            .bind(year)
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch unpublished dividend/payable date for specified year")?;
        Ok(rows)
    }

    /// 更新股利發放日期相關資訊（除息日、除權日、發放日）。
    async fn update_dividend_date(&self, dividend: &Dividend) -> Result<()> {
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
    async fn fetch_dividends_summary_by_date(
        &self,
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
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch dividends summary by date")?;
        Ok(rows)
    }

    /// 取得計算盈餘分配率所需的股利列與各期財報每股盈餘。
    ///
    /// 涵蓋期間要看同一所屬年度的其他配息才決定得了，因此整批取回後在記憶體計算
    /// （見 [`crate::domain::dividend::payout`]）；全市場約四萬多列股利、七萬列 EPS，
    /// 在正式機（樹莓派）上也只佔數 MB。不排序：計算結果與順序無關。
    async fn fetch_payout_ratio_inputs(
        &self,
    ) -> Result<(Vec<PayoutDividend>, Vec<PeriodEarnings>)> {
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
            .fetch_all(database::get_connection())
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
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch earnings for payout ratios")?;

        Ok((dividends, earnings))
    }

    /// 批次寫回計算好的盈餘分配率、分母 EPS 與涵蓋期間，回傳實際更新的列數。
    ///
    /// 以陣列參數一次更新，避免上千列各發一次 UPDATE；資料量再大也只有一次來回。
    async fn update_payout_ratios(&self, ratios: &[PayoutRatios]) -> Result<u64> {
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

    /// 取得指定日期有除權或除息事件的股票資料與參考收盤價。
    async fn fetch_stocks_with_dividends_on_date(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<DomainStockDividendInfo>> {
        let table_list = stock_dividend_info::fetch_stocks_with_dividends_on_date(date).await?;
        let domain_list = table_list
            .into_iter()
            .map(DomainStockDividendInfo::from)
            .collect();
        Ok(domain_list)
    }

    async fn fetch_payable_date_info_on_date(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<DomainStockDividendPayableDateInfo>> {
        use crate::infra::database::table::dividend::extension::stock_dividend_payable_date_info;
        let table_list = stock_dividend_payable_date_info::fetch(date).await?;
        let domain_list = table_list
            .into_iter()
            .map(DomainStockDividendPayableDateInfo::from)
            .collect();
        Ok(domain_list)
    }

    /// 儲存或更新股利；混合配息的全年明細改用 A，保留序號供持股紀錄關聯。
    ///
    /// 年度層級列（`quarter = ''`）以發放年度為鍵，分期明細以
    /// `(security_code, year, year_of_dividend, quarter)` 為鍵。分期明細若在同一發放年度、
    /// 同期別已有一列除權息日相同、只是所屬年度不同的資料，視為同一次配息的所屬年度更正，
    /// 先把那一列改成新的所屬年度再 upsert，避免同一次配息變成兩列。
    async fn save(&self, dividend: &Dividend) -> Result<()> {
        let mut tx = database::get_connection().begin().await?;
        if dividend.quarter == "A" {
            // 空季度原本同時代表年配事件與合計；先搬移事件，避免合計覆蓋它。
            // 搬移與 upsert 使用同一交易，失敗時不留下半套資料。
            sqlx::query(r#"
                UPDATE dividend SET quarter = 'A'
                WHERE security_code = $1 AND year = $2 AND year_of_dividend = $3
                  AND quarter = ''
                  AND NOT EXISTS (
                      SELECT 1 FROM dividend
                      WHERE security_code = $1 AND year = $2 AND year_of_dividend = $3 AND quarter = 'A'
                  )
            "#)
            .bind(&dividend.security_code)
            .bind(dividend.year)
            .bind(dividend.year_of_dividend)
            .execute(&mut *tx)
            .await?;
        }
        if !dividend.quarter.is_empty() {
            sqlx::query(RELABEL_SAME_EVENT_SQL)
                .bind(&dividend.security_code)
                .bind(dividend.year)
                .bind(dividend.year_of_dividend)
                .bind(&dividend.quarter)
                .bind(&dividend.ex_dividend_date_cash)
                .bind(&dividend.ex_dividend_date_stock)
                .execute(&mut *tx)
                .await
                .context("Failed to relabel the year of dividend")?;
        }
        let sql = if dividend.quarter.is_empty() {
            SAVE_ANNUAL_LEVEL_SQL
        } else {
            SAVE_DETAIL_SQL
        };
        sqlx::query(sql)
            .bind(&dividend.security_code)
            .bind(dividend.year)
            .bind(dividend.year_of_dividend)
            .bind(&dividend.quarter)
            .bind(dividend.cash_dividend)
            .bind(dividend.stock_dividend)
            .bind(dividend.sum)
            .bind(&dividend.ex_dividend_date_cash)
            .bind(&dividend.ex_dividend_date_stock)
            .bind(&dividend.payable_date_cash)
            .bind(&dividend.payable_date_stock)
            .bind(dividend.created_time)
            .bind(Local::now()) // updated_time
            .bind(dividend.capital_reserve_cash_dividend)
            .bind(dividend.earnings_cash_dividend)
            .bind(dividend.capital_reserve_stock_dividend)
            .bind(dividend.earnings_stock_dividend)
            .bind(dividend.payout_ratio_cash)
            .bind(dividend.payout_ratio_stock)
            .bind(dividend.payout_ratio)
            .execute(&mut *tx)
            .await
            .context("Failed to save dividend to database")?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
        let mut mixed_quarters: Vec<String> =
            mixed.iter().map(|item| item.quarter.clone()).collect();
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
}
