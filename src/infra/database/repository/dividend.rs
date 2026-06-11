use crate::domain::dividend::entity::{
    Dividend, StockDividendInfo as DomainStockDividendInfo,
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
use sqlx::{postgres::PgRow, Row};

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

    /// 取得尚未有指定年度配息的股票代號。
    async fn fetch_no_dividends_for_year(&self, year: i32) -> Result<Vec<String>> {
        let sql = r#"
            SELECT stock_symbol
            FROM stocks
            WHERE "SuspendListing" = false
                AND stock_exchange_market_id IN (2, 4)
                AND stock_symbol NOT IN 
                    (SELECT security_code FROM dividend WHERE year = $1 AND quarter = '');
        "#;
        let stock_symbols: Vec<String> = sqlx::query(sql)
            .bind(year)
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

    /// 合併並更新指定股票在指定發放年度的年度股利合計。
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
                cash_dividend = EXCLUDED.cash_dividend,
                stock_dividend = EXCLUDED.stock_dividend,
                sum = EXCLUDED.sum;
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

    /// 取得指定年度尚未有配息日或發放日的股息數據。
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
            FROM dividend
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
            FROM dividend
            WHERE security_code = $1
                AND year = $2
                AND ("ex-dividend_date1" <= $3)
                AND ("ex-dividend_date1" >= $4 OR "ex-dividend_date2" >= $4)
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

    /// 取得所有尚未計算或更新配息率的股利資料。
    async fn fetch_without_payout_ratio(&self) -> Result<Vec<Dividend>> {
        let sql = r#"
            SELECT 
                serial, security_code, year, year_of_dividend, quarter,
                cash_dividend, stock_dividend, sum, "ex-dividend_date1", "ex-dividend_date2",
                payable_date1, payable_date2, created_time, updated_time,
                capital_reserve_cash_dividend, earnings_cash_dividend,
                capital_reserve_stock_dividend, earnings_stock_dividend,
                payout_ratio_cash, payout_ratio_stock, payout_ratio
            FROM dividend
            WHERE payout_ratio = 0
                AND (cash_dividend > 0 OR stock_dividend > 0)
            ORDER BY year_of_dividend DESC
        "#;

        let rows = sqlx::query(sql)
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch dividends without payout ratio")?;
        Ok(rows)
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

    /// 儲存或更新單筆股利實體。
    async fn save(&self, dividend: &Dividend) -> Result<()> {
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
                payout_ratio_cash = EXCLUDED.payout_ratio_cash,
                payout_ratio_stock = EXCLUDED.payout_ratio_stock,
                payout_ratio = EXCLUDED.payout_ratio;
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
            .execute(database::get_connection())
            .await
            .context("Failed to save dividend to database")?;
        Ok(())
    }
}
