//! 櫃買中心個股日成交資訊（`afterTrading/tradingStock`）。
//!
//! 對應 TWSE 的 [`crate::infra::crawler::twse::stock_day`]：一次取「某一檔上櫃股票的整個月」，
//! 用來回補上櫃證券（主要是債券 ETF）的歷史日報價缺口。TWSE `STOCK_DAY` 只有上市證券，
//! 上櫃的缺口只能從這裡補。
//!
//! 數量單位與 TWSE 不同：成交量是「仟股」（近年欄名改為「成交張數」，一張同樣是一千股），
//! 成交金額是「仟元」，寫入前都要乘上一千換成股數與元，才會和每日收盤流程寫入的資料同單位。
//! 來源註明不含鉅額交易，和每日收盤流程的一般交易數字會有些微差距，開高低收則一致。

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    core::util::{
        datetime::{parse_taiwan_date, roc_year_to_gregorian_year},
        http, text,
    },
    infra::crawler::{share::DailyQuoteDto, tpex},
};

/// `tradingStock` 的回應主體。
#[derive(Serialize, Deserialize, Debug)]
pub struct TradingStockResponse {
    /// API 回應狀態；正常為小寫的 `"ok"`，查無資料時同樣是 `"ok"` 但 `data` 為空。
    pub stat: Option<String>,
    /// 資料表；個股日成交資訊只有一張。
    pub tables: Option<Vec<TradingStockTable>>,
}

/// `tradingStock` 回應中的資料表。
#[derive(Serialize, Deserialize, Debug)]
pub struct TradingStockTable {
    /// 欄位名稱：日期、成交仟股（成交張數）、成交仟元、開盤、最高、最低、收盤、漲跌、筆數。
    pub fields: Option<Vec<String>>,
    /// 資料列，欄位順序同 `fields`。
    pub data: Option<Vec<Vec<String>>>,
}

/// 資料列的欄位索引。
const COL_DATE: usize = 0;
const COL_TRADING_VOLUME: usize = 1;
const COL_TRADE_VALUE: usize = 2;
const COL_OPENING: usize = 3;
const COL_HIGHEST: usize = 4;
const COL_LOWEST: usize = 5;
const COL_CLOSING: usize = 6;
const COL_CHANGE: usize = 7;
const COL_TRANSACTION: usize = 8;
/// 一列至少要有到「筆數」為止的欄位才算完整。
const MIN_COLUMNS: usize = COL_TRANSACTION + 1;
/// 成交量（仟股／張）與成交金額（仟元）換成股數與元的倍數。
const THOUSAND: Decimal = Decimal::from_parts(1000, 0, 0, false, 0);

/// 抓取單一上櫃證券在指定月份的每日成交資訊。
///
/// `month` 只取其年月。查無資料（例如該證券當月尚未掛牌）時回傳空陣列而非錯誤。
pub async fn visit(stock_symbol: &str, month: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
    let url = format!(
        "https://{}/www/zh-tw/afterTrading/tradingStock?code={}&date={}&response=json",
        tpex::HOST,
        stock_symbol,
        month.format("%Y/%m/01")
    );

    let response = http::get_json::<TradingStockResponse>(&url)
        .await
        .with_context(|| {
            format!(
                "Failed to fetch TPEx tradingStock for {stock_symbol} at {}",
                month.format("%Y-%m")
            )
        })?;

    Ok(parse_trading_stock_response(&response, stock_symbol))
}

/// 將回應轉為每日報價。
///
/// 與 TWSE 版相同，刻意不回傳 `Result`：單一列解析失敗只跳過該列。`stat` 非 `ok` 時視為查無資料。
pub fn parse_trading_stock_response(
    response: &TradingStockResponse,
    stock_symbol: &str,
) -> Vec<DailyQuoteDto> {
    if !response
        .stat
        .as_deref()
        .is_some_and(|stat| stat.eq_ignore_ascii_case("ok"))
    {
        return Vec::new();
    }
    let Some(rows) = response
        .tables
        .as_ref()
        .and_then(|tables| tables.first())
        .and_then(|table| table.data.as_ref())
    else {
        return Vec::new();
    };

    let mut quotes = Vec::with_capacity(rows.len());
    for row in rows {
        if row.len() < MIN_COLUMNS {
            continue;
        }
        let Some(date) = parse_roc_date(&row[COL_DATE]) else {
            continue;
        };
        let Ok(closing_price) = number(&row[COL_CLOSING]) else {
            continue;
        };
        if closing_price <= Decimal::ZERO {
            continue;
        }

        let mut quote = DailyQuoteDto::new(stock_symbol.to_string(), date);
        quote.closing_price = closing_price;
        quote.opening_price = number(&row[COL_OPENING]).unwrap_or(closing_price);
        quote.highest_price = number(&row[COL_HIGHEST]).unwrap_or(closing_price);
        quote.lowest_price = number(&row[COL_LOWEST]).unwrap_or(closing_price);
        quote.trading_volume = number(&row[COL_TRADING_VOLUME]).unwrap_or_default() * THOUSAND;
        quote.trade_value = number(&row[COL_TRADE_VALUE]).unwrap_or_default() * THOUSAND;
        quote.transaction = number(&row[COL_TRANSACTION]).unwrap_or_default();
        quote.change = number(&row[COL_CHANGE]).unwrap_or_default();
        // 來源不提供漲跌幅，由漲跌回推前一日收盤再計算。
        let previous = closing_price - quote.change;
        if previous > Decimal::ZERO {
            quote.change_range = (quote.change / previous) * Decimal::ONE_HUNDRED;
        }

        quotes.push(quote);
    }

    quotes
}

/// [`crate::app::backfill::port::MonthlyQuoteFetcher`] 的櫃買中心實作。
#[derive(Debug, Default, Clone, Copy)]
pub struct TpexMonthlyQuoteFetcher;

#[async_trait::async_trait]
impl crate::app::backfill::port::MonthlyQuoteFetcher for TpexMonthlyQuoteFetcher {
    async fn fetch(&self, stock_symbol: &str, month: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
        visit(stock_symbol, month).await
    }
}

/// 解析民國年日期字串（`108/03/04`）。
///
/// 年份限定在民國 200 年以內：來源若改回西元（`2019/03/04`），不設限會算出西元 3930 年。
fn parse_roc_date(raw: &str) -> Option<NaiveDate> {
    /// 民國紀年的合理上界。
    const MAX_ROC_YEAR: i32 = 200;

    parse_taiwan_date(raw.trim())
        .filter(|date| date.year() <= roc_year_to_gregorian_year(MAX_ROC_YEAR))
}

/// 解析帶千分位逗號、正負號的數字；`--`、空字串等視為解析失敗。
fn number(raw: &str) -> Result<Decimal> {
    text::parse_decimal(raw, Some(vec![',', '+', 'X', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn fixture() -> TradingStockResponse {
        let raw = include_str!("testdata/trading_stock_00679B_201903.json");
        serde_json::from_str(raw).expect("fixture 應為合法 JSON")
    }

    fn row(date: &str, close: &str) -> Vec<String> {
        [
            date, "1,000", "10,000", "10.00", "10.50", "9.90", close, "+0.20", "3",
        ]
        .iter()
        .map(|value| (*value).to_owned())
        .collect()
    }

    fn response(stat: &str, rows: Vec<Vec<String>>) -> TradingStockResponse {
        TradingStockResponse {
            stat: Some(stat.to_owned()),
            tables: Some(vec![TradingStockTable {
                fields: None,
                data: Some(rows),
            }]),
        }
    }

    #[test]
    fn parse_maps_every_row_and_converts_thousand_units() {
        let quotes = parse_trading_stock_response(&fixture(), "00679B");
        assert_eq!(quotes.len(), 5);

        let first = &quotes[0];
        assert_eq!(first.symbol, "00679B");
        assert_eq!(
            first.date,
            NaiveDate::from_ymd_opt(2019, 3, 4).expect("日期")
        );
        assert_eq!(first.opening_price, dec!(37.96));
        assert_eq!(first.highest_price, dec!(38.09));
        assert_eq!(first.lowest_price, dec!(37.75));
        assert_eq!(first.closing_price, dec!(38.00));
        assert_eq!(first.change, dec!(-0.69));
        // 成交仟股與成交仟元要乘一千，才和每日收盤流程寫入的股數、元同單位。
        assert_eq!(first.trading_volume, dec!(1134000));
        assert_eq!(first.trade_value, dec!(43122000));
        assert_eq!(first.transaction, dec!(370));

        let expected = (dec!(-0.69) / dec!(38.69)) * Decimal::ONE_HUNDRED;
        assert_eq!(first.change_range, expected);
    }

    #[test]
    fn stat_is_compared_case_insensitively_and_other_values_mean_no_data() {
        assert_eq!(
            parse_trading_stock_response(
                &response("OK", vec![row("108/03/04", "10.20")]),
                "00679B"
            )
            .len(),
            1
        );
        assert!(
            parse_trading_stock_response(
                &response("error", vec![row("108/03/04", "10.20")]),
                "00679B"
            )
            .is_empty()
        );
        let missing_stat = TradingStockResponse {
            stat: None,
            tables: None,
        };
        assert!(parse_trading_stock_response(&missing_stat, "00679B").is_empty());
    }

    /// 尚未掛牌的月份：stat 仍是 ok，但資料表是空的。
    #[test]
    fn empty_table_is_treated_as_no_data() {
        assert!(parse_trading_stock_response(&response("ok", vec![]), "00679B").is_empty());
        let no_tables = TradingStockResponse {
            stat: Some("ok".to_owned()),
            tables: Some(vec![]),
        };
        assert!(parse_trading_stock_response(&no_tables, "00679B").is_empty());
    }

    #[test]
    fn malformed_rows_and_non_positive_closes_are_skipped() {
        let quotes = parse_trading_stock_response(
            &response(
                "ok",
                vec![
                    vec!["108/03/04".to_owned(), "1".to_owned()],
                    row("2019-03-05", "10.20"),
                    row("108/03/06", "--"),
                    row("108/03/07", "0.00"),
                    row("108/03/08", "10.30"),
                ],
            ),
            "00679B",
        );

        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].closing_price, dec!(10.30));
    }

    #[test]
    fn parse_roc_date_rejects_gregorian_years() {
        assert_eq!(
            parse_roc_date("108/03/04"),
            NaiveDate::from_ymd_opt(2019, 3, 4)
        );
        assert_eq!(
            parse_roc_date(" 99/01/04 "),
            NaiveDate::from_ymd_opt(2010, 1, 4)
        );
        assert_eq!(parse_roc_date("2019/03/04"), None);
        assert_eq!(parse_roc_date("108/13/04"), None);
        assert_eq!(parse_roc_date("--"), None);
    }

    /// 本益比、股價淨值比與最佳買賣揭示不在此來源，必須維持 0。
    #[test]
    fn fields_absent_from_this_source_stay_zero() {
        let quotes = parse_trading_stock_response(&fixture(), "00679B");
        let quote = &quotes[0];

        assert_eq!(quote.price_earning_ratio, Decimal::ZERO);
        assert_eq!(quote.price_to_book_ratio, Decimal::ZERO);
        assert_eq!(quote.last_best_bid_price, Decimal::ZERO);
        assert_eq!(quote.last_best_ask_price, Decimal::ZERO);
    }
}
