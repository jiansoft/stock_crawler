use crate::domain::dividend::entity::{
    Dividend, PayoutRatioCandidate, PayoutRatios, StockDividendInfo as DomainStockDividendInfo,
    StockDividendPayableDateInfo as DomainStockDividendPayableDateInfo,
};
use crate::domain::dividend::repository::DividendRepository;
use crate::infra::database;
use crate::infra::database::table::dividend::extension::stock_dividend_info::{
    self, StockDividendInfo as TableStockDividendInfo,
};
use crate::infra::database::table::dividend::extension::stock_dividend_payable_date_info::StockDividendPayableDateInfo as TableStockDividendPayableDateInfo;
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
    /// 的明細列會與新的合計列撞上同一組主鍵），留著舊日期會讓合計被下游當成真實的配息事件。
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
            ON CONFLICT (security_code,year,quarter) DO UPDATE SET
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

    /// 取得待計算盈餘分配率的股利列，並帶出同期間的每股盈餘。
    ///
    /// 每股盈餘依股利的所屬期間對應到 `financial_statement`：
    /// 年度列（空季別與混合年度的 `A`）優先取年度財報，沒有才用四季加總；
    /// 半年配對應上／下半年兩季的加總；季配直接對應同一季。
    /// 加總一律要求該期間的季別到齊，避免財報只出一半就算出偏低的分配率。
    ///
    /// 不排序：結果整批取回後在記憶體計算，順序不影響任何結果，省掉一次 Sort。
    async fn fetch_payout_ratio_candidates(&self) -> Result<Vec<PayoutRatioCandidate>> {
        let sql = r#"
            SELECT
                d.serial,
                d.cash_dividend,
                d.stock_dividend,
                d."sum",
                CASE
                    WHEN d.quarter IN ('', 'A') THEN COALESCE(
                        (SELECT fs.earnings_per_share
                           FROM financial_statement AS fs
                          WHERE fs.security_code = d.security_code
                            AND fs.year = d.year_of_dividend
                            AND fs.quarter = ''),
                        (SELECT SUM(fs.earnings_per_share)
                           FROM financial_statement AS fs
                          WHERE fs.security_code = d.security_code
                            AND fs.year = d.year_of_dividend
                            AND fs.quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
                         HAVING COUNT(*) = 4)
                    )
                    WHEN d.quarter = 'H1' THEN
                        (SELECT SUM(fs.earnings_per_share)
                           FROM financial_statement AS fs
                          WHERE fs.security_code = d.security_code
                            AND fs.year = d.year_of_dividend
                            AND fs.quarter IN ('Q1', 'Q2')
                         HAVING COUNT(*) = 2)
                    WHEN d.quarter = 'H2' THEN
                        (SELECT SUM(fs.earnings_per_share)
                           FROM financial_statement AS fs
                          WHERE fs.security_code = d.security_code
                            AND fs.year = d.year_of_dividend
                            AND fs.quarter IN ('Q3', 'Q4')
                         HAVING COUNT(*) = 2)
                    ELSE
                        (SELECT fs.earnings_per_share
                           FROM financial_statement AS fs
                          WHERE fs.security_code = d.security_code
                            AND fs.year = d.year_of_dividend
                            AND fs.quarter = d.quarter)
                END AS earnings_per_share
            FROM dividend AS d
            WHERE d.payout_ratio = 0
              AND d."sum" > 0
              -- 資料表裡有 year_of_dividend = 0 的殘留列，財報那邊也有 year = 0 的髒資料，
              -- 兩者會互相 join 出沒有意義的分配率。
              AND d.year_of_dividend > 0
              -- ETF 與債券只配息、沒有財報，分配率永遠算不出來。
              -- 先擋掉整批沒有財報的標的，才不會每天都把它們撈出來跑五個子查詢。
              -- 用 EXISTS 而不是硬編碼代號規則：新標的等財報進來就會自動納入。
              AND EXISTS (
                  SELECT 1
                    FROM financial_statement AS fs
                   WHERE fs.security_code = d.security_code
              )
        "#;

        let rows = sqlx::query(sql)
            .try_map(|row: PgRow| {
                Ok(PayoutRatioCandidate {
                    serial: row.try_get("serial")?,
                    cash_dividend: row.try_get("cash_dividend")?,
                    stock_dividend: row.try_get("stock_dividend")?,
                    sum: row.try_get("sum")?,
                    earnings_per_share: row.try_get("earnings_per_share")?,
                })
            })
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch payout ratio candidates")?;

        Ok(rows)
    }

    /// 批次寫回計算好的盈餘分配率，回傳實際更新的列數。
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

        let sql = r#"
            UPDATE dividend AS d
            SET payout_ratio_cash = u.payout_ratio_cash,
                payout_ratio_stock = u.payout_ratio_stock,
                payout_ratio = u.payout_ratio,
                updated_time = now()
            FROM UNNEST($1::bigint[], $2::numeric[], $3::numeric[], $4::numeric[])
                AS u(serial, payout_ratio_cash, payout_ratio_stock, payout_ratio)
            WHERE d.serial = u.serial
        "#;

        let result = sqlx::query(sql)
            .bind(&serials)
            .bind(&cash)
            .bind(&stock)
            .bind(&total)
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
                      SELECT 1 FROM dividend WHERE security_code = $1 AND year = $2 AND quarter = 'A'
                  )
            "#)
            .bind(&dividend.security_code)
            .bind(dividend.year)
            .bind(dividend.year_of_dividend)
            .execute(&mut *tx)
            .await?;
        }
        let sql = r#"
            INSERT INTO dividend (
                security_code, "year", year_of_dividend, quarter,
                cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2",
                payable_date1, payable_date2, created_time, updated_time, capital_reserve_cash_dividend,
                earnings_cash_dividend, capital_reserve_stock_dividend, earnings_stock_dividend,
                payout_ratio_cash, payout_ratio_stock, payout_ratio)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
            ON CONFLICT (security_code, "year", quarter) DO UPDATE SET
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
        "#;
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
}
