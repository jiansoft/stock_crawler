//! TWSE 個股日成交資訊（`STOCK_DAY`）。
//!
//! 與 [`super::quote`] 的差異：`MI_INDEX` 一次取「某一交易日的全市場」，
//! 這裡一次取「某一檔股票的整個月」。回補歷史缺口時後者才實用 ——
//! 缺七年份就得逐日呼叫一千七百多次 `MI_INDEX`，而且會把不缺的股票也一併重抓。
//!
//! 證交所說明此資料自 2010-01-04 起提供。

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    core::util::{datetime::roc_year_to_gregorian_year, http, text},
    infra::crawler::{share::DailyQuoteDto, twse},
};

/// `STOCK_DAY` 的回應主體。
#[derive(Serialize, Deserialize, Debug)]
pub struct StockDayResponse {
    /// API 回應狀態；查無資料時不是 `"OK"`。
    pub stat: Option<String>,
    /// 標題，形如 `110年08月 0050 元大台灣50 各日成交資訊`。
    pub title: Option<String>,
    /// 欄位名稱清單。
    pub fields: Option<Vec<String>>,
    /// 資料列：日期、成交股數、成交金額、開盤價、最高價、最低價、收盤價、
    /// 漲跌價差、成交筆數、註記。
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
/// 一列至少要有到「成交筆數」為止的欄位才算完整。
const MIN_COLUMNS: usize = COL_TRANSACTION + 1;

/// 抓取單一股票在指定月份的每日成交資訊。
///
/// `month` 只取其年月，日期部分由 API 忽略。查無資料（例如該股當月尚未上市）
/// 時回傳空陣列而非錯誤 —— 回補整段區間時，中間有幾個月沒有資料是常態。
pub async fn visit(stock_symbol: &str, month: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
    let url = format!(
        "https://www.{}/exchangeReport/STOCK_DAY?response=json&date={}&stockNo={}&_={}",
        twse::HOST,
        month.format("%Y%m01"),
        stock_symbol,
        Local::now().timestamp_millis()
    );

    let response = http::get_json::<StockDayResponse>(&url)
        .await
        .with_context(|| {
            format!(
                "Failed to fetch STOCK_DAY for {stock_symbol} at {}",
                month.format("%Y-%m")
            )
        })?;

    Ok(parse_stock_day_response(&response, stock_symbol))
}

/// 將回應轉為每日報價。
///
/// 刻意不回傳 `Result`：單一列解析失敗（無成交的 `--`、格式異動）只跳過該列，
/// 不讓整個月的資料連帶消失。`stat` 非 `OK` 時視為查無資料。
pub fn parse_stock_day_response(
    response: &StockDayResponse,
    stock_symbol: &str,
) -> Vec<DailyQuoteDto> {
    if response.stat.as_deref() != Some("OK") {
        return Vec::new();
    }
    let Some(rows) = response.data.as_ref() else {
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
        // 收盤價是後續所有計算的基礎；當日無成交時 TWSE 會給 `--`。
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
        quote.trading_volume = number(&row[COL_TRADING_VOLUME]).unwrap_or_default();
        quote.trade_value = number(&row[COL_TRADE_VALUE]).unwrap_or_default();
        quote.transaction = number(&row[COL_TRANSACTION]).unwrap_or_default();
        quote.change = number(&row[COL_CHANGE]).unwrap_or_default();
        // 漲跌幅由漲跌價差回推：STOCK_DAY 不提供百分比，而前一日收盤價
        // 未必在本次結果內（月初第一筆）。
        let previous = closing_price - quote.change;
        if previous > Decimal::ZERO {
            quote.change_range = (quote.change / previous) * Decimal::ONE_HUNDRED;
        }
        // 本益比、股價淨值比與最佳買賣揭示不在此來源，維持預設值 0。

        quotes.push(quote);
    }

    quotes
}

/// 解析民國年日期字串（`110/08/02`）。
///
/// 年份限定在合理的民國紀年範圍內：若來源哪天改回西元（`2021/08/02`），
/// 不設限會安靜地算出民國 2021 年＝西元 3932 年這種荒謬日期並寫進資料庫。
fn parse_roc_date(raw: &str) -> Option<NaiveDate> {
    /// 民國紀年的合理上界；本專案的報價資料不可能超出這個範圍。
    const MAX_ROC_YEAR: i32 = 200;

    let mut parts = raw.trim().split('/');
    let roc_year = parts.next()?.trim().parse::<i32>().ok()?;
    if !(1..=MAX_ROC_YEAR).contains(&roc_year) {
        return None;
    }
    let month = parts.next()?.trim().parse::<u32>().ok()?;
    let day = parts.next()?.trim().parse::<u32>().ok()?;
    NaiveDate::from_ymd_opt(roc_year_to_gregorian_year(roc_year), month, day)
}

/// 解析帶千分位逗號、正負號的數字；`--`、空字串等視為解析失敗。
fn number(raw: &str) -> Result<Decimal> {
    text::parse_decimal(raw, Some(vec![',', '+', 'X', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn fixture() -> StockDayResponse {
        let raw = include_str!("testdata/stock_day_0050_202108.json");
        serde_json::from_str(raw).expect("fixture 應為合法 JSON")
    }

    #[test]
    fn parse_stock_day_response_maps_every_traded_day() {
        let quotes = parse_stock_day_response(&fixture(), "0050");

        // fixture 共六列，其中一列是無成交的 `--`，必須被略過。
        assert_eq!(quotes.len(), 5);

        let first = &quotes[0];
        assert_eq!(first.symbol, "0050");
        assert_eq!(
            first.date,
            NaiveDate::from_ymd_opt(2021, 8, 2).expect("測試日期應合法")
        );
        assert_eq!(first.opening_price, dec!(137.00));
        assert_eq!(first.highest_price, dec!(138.00));
        assert_eq!(first.lowest_price, dec!(136.05));
        assert_eq!(first.closing_price, dec!(137.90));
        assert_eq!(first.change, dec!(1.85));
        // 千分位逗號必須去掉，否則成交股數會解析失敗而變成 0。
        assert_eq!(first.trading_volume, dec!(7376244));
        assert_eq!(first.trade_value, dec!(1011342122));
        assert_eq!(first.transaction, dec!(5273));
    }

    #[test]
    fn change_range_is_derived_from_the_previous_close() {
        let quotes = parse_stock_day_response(&fixture(), "0050");
        let first = &quotes[0];

        // 前一日收盤 = 137.90 − 1.85 = 136.05，漲幅約 1.36%。
        let expected = (dec!(1.85) / dec!(136.05)) * Decimal::ONE_HUNDRED;
        assert_eq!(first.change_range, expected);
    }

    #[test]
    fn negative_change_is_parsed_with_its_sign() {
        let quotes = parse_stock_day_response(&fixture(), "0050");
        let down_day = quotes
            .iter()
            .find(|quote| quote.date == NaiveDate::from_ymd_opt(2021, 8, 5).expect("日期"))
            .expect("2021-08-05 應在結果中");

        assert_eq!(down_day.change, dec!(-0.25));
        assert!(down_day.change_range < Decimal::ZERO);
    }

    #[test]
    fn non_ok_status_is_treated_as_no_data() {
        let response = StockDayResponse {
            stat: Some("很抱歉，沒有符合條件的資料!".to_owned()),
            title: None,
            fields: None,
            data: None,
        };
        assert!(parse_stock_day_response(&response, "0050").is_empty());
    }

    #[test]
    fn malformed_rows_are_skipped_without_dropping_the_month() {
        let response = StockDayResponse {
            stat: Some("OK".to_owned()),
            title: None,
            fields: None,
            data: Some(vec![
                // 欄位不足。
                vec!["110/08/02".to_owned(), "1".to_owned()],
                // 日期格式錯誤。
                vec![
                    "2021-08-03".to_owned(),
                    "1".to_owned(),
                    "1".to_owned(),
                    "10".to_owned(),
                    "10".to_owned(),
                    "10".to_owned(),
                    "10".to_owned(),
                    "0".to_owned(),
                    "1".to_owned(),
                ],
                // 正常列。
                vec![
                    "110/08/04".to_owned(),
                    "1,000".to_owned(),
                    "10,000".to_owned(),
                    "10.00".to_owned(),
                    "10.50".to_owned(),
                    "9.90".to_owned(),
                    "10.20".to_owned(),
                    "+0.20".to_owned(),
                    "3".to_owned(),
                ],
            ]),
        };

        let quotes = parse_stock_day_response(&response, "0050");
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].closing_price, dec!(10.20));
    }

    /// `data` 為 null（TWSE 偶爾如此回應）時視為查無資料，不可 panic。
    #[test]
    fn missing_data_array_is_treated_as_no_data() {
        let response = StockDayResponse {
            stat: Some("OK".to_owned()),
            title: None,
            fields: None,
            data: None,
        };
        assert!(parse_stock_day_response(&response, "0050").is_empty());

        // stat 缺漏同樣視為查無資料。
        let response = StockDayResponse {
            stat: None,
            title: None,
            fields: None,
            data: Some(vec![]),
        };
        assert!(parse_stock_day_response(&response, "0050").is_empty());
    }

    /// 收盤價為 `--` 或 0 的列必須整列跳過：0 元收盤會污染後續所有計算。
    #[test]
    fn rows_without_a_positive_close_are_skipped() {
        let row = |close: &str| {
            vec![
                "110/08/04".to_owned(),
                "1,000".to_owned(),
                "10,000".to_owned(),
                "10.00".to_owned(),
                "10.50".to_owned(),
                "9.90".to_owned(),
                close.to_owned(),
                "+0.20".to_owned(),
                "3".to_owned(),
            ]
        };
        let response = StockDayResponse {
            stat: Some("OK".to_owned()),
            title: None,
            fields: None,
            data: Some(vec![row("--"), row("0.00"), row("-1.00")]),
        };

        assert!(parse_stock_day_response(&response, "0050").is_empty());
    }

    /// 開高低價缺漏時回退為收盤價，其餘量值欄位回退為 0。
    #[test]
    fn missing_optional_columns_fall_back_to_safe_values() {
        let response = StockDayResponse {
            stat: Some("OK".to_owned()),
            title: None,
            fields: None,
            data: Some(vec![vec![
                "110/08/04".to_owned(),
                "--".to_owned(),
                "--".to_owned(),
                "--".to_owned(),
                "--".to_owned(),
                "--".to_owned(),
                "10.20".to_owned(),
                "--".to_owned(),
                "--".to_owned(),
            ]]),
        };

        let quotes = parse_stock_day_response(&response, "0050");
        assert_eq!(quotes.len(), 1);
        let quote = &quotes[0];
        assert_eq!(quote.opening_price, dec!(10.20));
        assert_eq!(quote.highest_price, dec!(10.20));
        assert_eq!(quote.lowest_price, dec!(10.20));
        assert_eq!(quote.trading_volume, Decimal::ZERO);
        assert_eq!(quote.trade_value, Decimal::ZERO);
        assert_eq!(quote.transaction, Decimal::ZERO);
        assert_eq!(quote.change, Decimal::ZERO);
        // 漲跌為 0 時前一日收盤等於本日收盤，漲跌幅為 0。
        assert_eq!(quote.change_range, Decimal::ZERO);
    }

    /// 本益比、股價淨值比與最佳買賣揭示不在此來源，必須維持 0 而非亂填。
    #[test]
    fn fields_absent_from_this_source_stay_zero() {
        let quotes = parse_stock_day_response(&fixture(), "0050");
        let quote = &quotes[0];

        assert_eq!(quote.price_earning_ratio, Decimal::ZERO);
        assert_eq!(quote.price_to_book_ratio, Decimal::ZERO);
        assert_eq!(quote.last_best_bid_price, Decimal::ZERO);
        assert_eq!(quote.last_best_ask_price, Decimal::ZERO);
    }

    /// 千分位、正負號與空白都要被吃掉；`--` 與空字串則視為解析失敗。
    #[test]
    fn number_strips_formatting_and_rejects_placeholders() {
        assert_eq!(number("1,011,342,122").expect("千分位"), dec!(1011342122));
        assert_eq!(number("+0.20").expect("正號"), dec!(0.20));
        assert_eq!(number("-0.25").expect("負號"), dec!(-0.25));
        for raw in ["--", "", "   ", "N/A"] {
            assert!(number(raw).is_err(), "raw={raw:?} 應解析失敗");
        }
    }

    #[test]
    fn parse_roc_date_converts_from_minguo_calendar() {
        assert_eq!(
            parse_roc_date("110/08/02"),
            NaiveDate::from_ymd_opt(2021, 8, 2)
        );
        assert_eq!(
            parse_roc_date(" 99/01/04 "),
            NaiveDate::from_ymd_opt(2010, 1, 4)
        );
        assert_eq!(parse_roc_date("2021/08/02"), None);
        assert_eq!(parse_roc_date("110/13/02"), None);
        assert_eq!(parse_roc_date("--"), None);
    }
}
