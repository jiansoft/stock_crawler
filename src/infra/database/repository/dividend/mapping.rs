//! 股利倉儲的資料列轉換：資料表擴充查詢結果轉成領域型別、`PgRow` 轉成 [`Dividend`]。

use sqlx::{Row, postgres::PgRow};

use super::PgDividendRepository;
use crate::domain::dividend::entity::{
    Dividend, StockDividendInfo as DomainStockDividendInfo,
    StockDividendPayableDateInfo as DomainStockDividendPayableDateInfo,
};
use crate::infra::database::table::dividend::extension::stock_dividend_info::StockDividendInfo as TableStockDividendInfo;
use crate::infra::database::table::dividend::extension::stock_dividend_payable_date_info::StockDividendPayableDateInfo as TableStockDividendPayableDateInfo;

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
            quarter: table.quarter,
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

impl PgDividendRepository {
    /// 將資料庫的 `PgRow` 轉換成領域實體 `Dividend`。
    pub(super) fn row_to_entity(row: PgRow) -> Result<Dividend, sqlx::Error> {
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
