//! # 每日收盤報價載體
//!
//! 定義每日收盤報價的爬蟲資料載體 (DTO)，以及兩種來源格式的轉換邏輯：
//! 以「欄位名稱對照表」對應欄位（TWSE MI_INDEX 等欄位順序會變動的來源），
//! 以及以「固定欄位順序」對應欄位（TPEx 收盤行情等）。
//! 實際的欄位數值解析規則集中在 [`super::quote_field`]。

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::quote_field::{QuoteParseError, parse_quote_decimal, parse_soft_quote_decimal};
use crate::core::declare::StockExchange;

/// 每日收盤報價爬蟲載體 (DTO)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyQuoteDto {
    /// 股票代號
    pub symbol: String,
    /// 交易日期
    pub date: NaiveDate,
    /// 開盤價
    pub opening_price: Decimal,
    /// 最高價
    pub highest_price: Decimal,
    /// 最低價
    pub lowest_price: Decimal,
    /// 收盤價
    pub closing_price: Decimal,
    /// 漲跌價差
    pub change: Decimal,
    /// 漲跌幅（百分比）
    pub change_range: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    /// 成交金額
    pub trade_value: Decimal,
    /// 成交筆數
    pub transaction: Decimal,
    /// 本益比
    pub price_earning_ratio: Decimal,
    /// 股價淨值比
    pub price_to_book_ratio: Decimal,
    /// 最後揭示買價
    pub last_best_bid_price: Decimal,
    /// 最後揭示買量
    pub last_best_bid_volume: Decimal,
    /// 最後揭示賣價
    pub last_best_ask_price: Decimal,
    /// 最後揭示賣量
    pub last_best_ask_volume: Decimal,
}

impl DailyQuoteDto {
    /// 建立 `DailyQuoteDto` 預設實例，並同步初始化代碼與日期。
    pub fn new<S: Into<String>>(symbol: S, date: NaiveDate) -> Self {
        Self {
            symbol: symbol.into(),
            date,
            opening_price: Decimal::ZERO,
            highest_price: Decimal::ZERO,
            lowest_price: Decimal::ZERO,
            closing_price: Decimal::ZERO,
            change: Decimal::ZERO,
            change_range: Decimal::ZERO,
            trading_volume: Decimal::ZERO,
            trade_value: Decimal::ZERO,
            transaction: Decimal::ZERO,
            price_earning_ratio: Decimal::ZERO,
            price_to_book_ratio: Decimal::ZERO,
            last_best_bid_price: Decimal::ZERO,
            last_best_bid_volume: Decimal::ZERO,
            last_best_ask_price: Decimal::ZERO,
            last_best_ask_volume: Decimal::ZERO,
        }
    }

    /// 依欄位名稱映射，從單筆原始字串資料建立 `DailyQuoteDto`。
    ///
    /// `map` 是「欄位名稱 → 欄位索引」的對照表（由呼叫端從來源的表頭建立），
    /// 適用於欄位順序可能變動的來源（例如 TWSE MI_INDEX）。
    ///
    /// # Errors
    ///
    /// - 任何必要欄位在 `map` 中不存在、或該列長度不足時，
    ///   回傳 [`QuoteParseError::MissingField`]（通常代表來源改了欄位名稱）。
    /// - 欄位內容不是數字、也不是 `--` 等「無資料」佔位符時，
    ///   回傳 [`QuoteParseError::InvalidDecimal`]。
    ///
    /// 舊版對上述兩種情況都默默補 0，導致資料庫可能寫入「部分欄位為零」的
    /// 半套行情；現在改為把整列拒絕，由呼叫端統計拒絕比例決定後續處理。
    pub fn from_with_map(
        item: &[String],
        map: &std::collections::HashMap<&str, usize>,
        date: NaiveDate,
    ) -> Result<Self, QuoteParseError> {
        // 依欄位名稱取出原始字串；欄位不存在或索引超界都視為結構性錯誤。
        let get_field = |field: &'static str| -> Result<&String, QuoteParseError> {
            map.get(field)
                .and_then(|&i| item.get(i))
                .ok_or(QuoteParseError::MissingField { field })
        };

        let code = get_field("證券代號")?.clone();
        let mut dto = DailyQuoteDto::new(code, date);

        // 逐一解析數值欄位；任何一欄失敗都會讓整列被拒絕（? 提早返回）。
        dto.trading_volume = parse_quote_decimal("成交股數", get_field("成交股數")?)?;
        dto.transaction = parse_quote_decimal("成交筆數", get_field("成交筆數")?)?;
        dto.trade_value = parse_quote_decimal("成交金額", get_field("成交金額")?)?;
        dto.opening_price = parse_quote_decimal("開盤價", get_field("開盤價")?)?;
        dto.highest_price = parse_quote_decimal("最高價", get_field("最高價")?)?;
        dto.lowest_price = parse_quote_decimal("最低價", get_field("最低價")?)?;
        dto.closing_price = parse_quote_decimal("收盤價", get_field("收盤價")?)?;
        // 漲跌價差採「軟性」解析：除權息日此欄可能是非數字標記，補 0 不拒絕整列。
        dto.change = parse_soft_quote_decimal("漲跌價差", get_field("漲跌價差")?);
        dto.last_best_bid_price = parse_quote_decimal("最後揭示買價", get_field("最後揭示買價")?)?;
        dto.last_best_bid_volume = parse_quote_decimal("最後揭示買量", get_field("最後揭示買量")?)?;
        dto.last_best_ask_price = parse_quote_decimal("最後揭示賣價", get_field("最後揭示賣價")?)?;
        dto.last_best_ask_volume = parse_quote_decimal("最後揭示賣量", get_field("最後揭示賣量")?)?;
        dto.price_earning_ratio = parse_quote_decimal("本益比", get_field("本益比")?)?;

        // 處理漲跌符號。TWSE 把「漲/跌」方向放在獨立欄位（+/- 或紅/綠字），
        // 漲跌價差本身是無號數。這個欄位若被改名而遺失，漲跌方向會整批出錯，
        // 因此也視為必要欄位。
        let sign = get_field("漲跌(+/-)")?;
        if sign.contains('-') || sign.contains('綠') {
            dto.change = -dto.change.abs();
        } else if sign.contains('+') || sign.contains('紅') {
            dto.change = dto.change.abs();
        }

        Ok(dto)
    }

    /// 在給定交易所與日期的前提下，將「固定欄位順序」的來源資料轉成 `DailyQuoteDto`。
    ///
    /// 與 [`Self::from_with_map`] 的差別：這裡的來源（例如 TPEx 收盤行情）以
    /// 位置（索引）而不是欄位名稱對應欄位。
    ///
    /// # Errors
    ///
    /// 該列長度不足（缺欄位）回傳 [`QuoteParseError::MissingField`]；
    /// 欄位內容無法解析且非「無資料」佔位符時回傳 [`QuoteParseError::InvalidDecimal`]。
    pub fn from_with_exchange(
        exchange: StockExchange,
        item: &[String],
        date: NaiveDate,
    ) -> Result<Self, QuoteParseError> {
        // 第 0 欄固定是證券代號；整列為空時直接視為缺欄位。
        let symbol = item.first().ok_or(QuoteParseError::MissingField {
            field: "證券代號"
        })?;
        let mut dto = DailyQuoteDto::new(symbol.to_string(), date);

        match exchange {
            StockExchange::TWSE => {
                // (索引, 欄位名稱)。欄位名稱只用於錯誤訊息，讓除錯時能對照來源表頭。
                let decimal_fields = [
                    (2, "成交股數", &mut dto.trading_volume),
                    (3, "成交筆數", &mut dto.transaction),
                    (4, "成交金額", &mut dto.trade_value),
                    (5, "開盤價", &mut dto.opening_price),
                    (6, "最高價", &mut dto.highest_price),
                    (7, "最低價", &mut dto.lowest_price),
                    (8, "收盤價", &mut dto.closing_price),
                    (11, "最後揭示買價", &mut dto.last_best_bid_price),
                    (12, "最後揭示買量", &mut dto.last_best_bid_volume),
                    (13, "最後揭示賣價", &mut dto.last_best_ask_price),
                    (14, "最後揭示賣量", &mut dto.last_best_ask_volume),
                    (15, "本益比", &mut dto.price_earning_ratio),
                ];

                for (index, field, target) in decimal_fields {
                    let raw = item
                        .get(index)
                        .ok_or(QuoteParseError::MissingField { field })?;
                    *target = parse_quote_decimal(field, raw)?;
                }

                // 第 10 欄是漲跌價差，採「軟性」解析：除權息日此欄可能是
                // 非數字標記，補 0 不拒絕整列（缺欄位仍是硬錯誤）。
                let change_raw = item.get(10).ok_or(QuoteParseError::MissingField {
                    field: "漲跌價差",
                })?;
                dto.change = parse_soft_quote_decimal("漲跌價差", change_raw);

                // 第 9 欄是漲跌方向（HTML 內含 + 或 -）；含 '-' 時把漲跌價差轉負。
                let sign = item.get(9).ok_or(QuoteParseError::MissingField {
                    field: "漲跌(+/-)",
                })?;
                if sign.contains('-') {
                    dto.change = -dto.change;
                }
            }
            StockExchange::TPEx => {
                // TPEx 的漲跌欄（索引 3）自帶正負號，不需要獨立的方向欄位。
                let decimal_fields = [
                    (7, "成交股數", &mut dto.trading_volume),
                    (9, "成交筆數", &mut dto.transaction),
                    (8, "成交金額", &mut dto.trade_value),
                    (4, "開盤價", &mut dto.opening_price),
                    (5, "最高價", &mut dto.highest_price),
                    (6, "最低價", &mut dto.lowest_price),
                    (2, "收盤價", &mut dto.closing_price),
                    (10, "最後揭示買價", &mut dto.last_best_bid_price),
                    (11, "最後揭示買量", &mut dto.last_best_bid_volume),
                    (12, "最後揭示賣價", &mut dto.last_best_ask_price),
                    (13, "最後揭示賣量", &mut dto.last_best_ask_volume),
                ];

                for (index, field, target) in decimal_fields {
                    let raw = item
                        .get(index)
                        .ok_or(QuoteParseError::MissingField { field })?;
                    *target = parse_quote_decimal(field, raw)?;
                }

                // 第 3 欄是自帶正負號的漲跌，採「軟性」解析：
                // 除權息日此欄可能是非數字標記，補 0 不拒絕整列。
                let change_raw = item
                    .get(3)
                    .ok_or(QuoteParseError::MissingField { field: "漲跌" })?;
                dto.change = parse_soft_quote_decimal("漲跌", change_raw);
            }
            _ => {}
        }

        Ok(dto)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::core::declare::StockExchange;

    /// 建立 TWSE 欄位名稱對照表與一筆有效資料列，供 from_with_map 測試共用。
    fn twse_map_and_row() -> (HashMap<&'static str, usize>, Vec<String>) {
        let map = HashMap::from([
            ("證券代號", 0),
            ("成交股數", 1),
            ("成交筆數", 2),
            ("成交金額", 3),
            ("開盤價", 4),
            ("最高價", 5),
            ("最低價", 6),
            ("收盤價", 7),
            ("漲跌(+/-)", 8),
            ("漲跌價差", 9),
            ("最後揭示買價", 10),
            ("最後揭示買量", 11),
            ("最後揭示賣價", 12),
            ("最後揭示賣量", 13),
            ("本益比", 14),
        ]);
        let row = vec![
            "2330".to_string(),
            "1,234,000".to_string(),
            "5,678".to_string(),
            "987,654,321".to_string(),
            "950.5".to_string(),
            "960.5".to_string(),
            "945.5".to_string(),
            "955.5".to_string(),
            "綠".to_string(),
            "12.5".to_string(),
            "955.0".to_string(),
            "100".to_string(),
            "956.0".to_string(),
            "200".to_string(),
            "20.5".to_string(),
        ];
        (map, row)
    }

    /// 驗證 from_with_map 正常解析（含千分位、綠字轉負值）。
    #[test]
    fn from_with_map_parses_valid_row() {
        let (map, row) = twse_map_and_row();
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let dto = DailyQuoteDto::from_with_map(&row, &map, date).unwrap();

        assert_eq!(dto.symbol, "2330");
        assert_eq!(dto.trading_volume, dec!(1234000));
        assert_eq!(dto.closing_price, dec!(955.5));
        // 「綠」代表下跌，漲跌價差應轉為負值。
        assert_eq!(dto.change, dec!(-12.5));
        assert_eq!(dto.price_earning_ratio, dec!(20.5));
    }

    /// 驗證欄位改名（對照表缺欄位）時整列被拒絕，而不是補 0。
    #[test]
    fn from_with_map_rejects_missing_column() {
        let (mut map, row) = twse_map_and_row();
        // 模擬來源把「收盤價」改名：對照表中不再有這個欄位。
        map.remove("收盤價");
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let err = DailyQuoteDto::from_with_map(&row, &map, date).unwrap_err();

        assert!(matches!(
            err,
            QuoteParseError::MissingField { field: "收盤價" }
        ));
    }

    /// 驗證價格欄位含垃圾內容時整列被拒絕。
    #[test]
    fn from_with_map_rejects_garbage_price() {
        let (map, mut row) = twse_map_and_row();
        row[7] = "corrupted".to_string(); // 收盤價
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let err = DailyQuoteDto::from_with_map(&row, &map, date).unwrap_err();

        assert!(matches!(err, QuoteParseError::InvalidDecimal { .. }));
    }

    /// 驗證漲跌價差是「軟性」欄位：非數字標記補 0，不拒絕整列。
    #[test]
    fn from_with_map_soft_change_falls_back_to_zero() {
        let (map, mut row) = twse_map_and_row();
        row[9] = "除息".to_string(); // 漲跌價差
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let dto = DailyQuoteDto::from_with_map(&row, &map, date).unwrap();

        assert_eq!(dto.change, Decimal::ZERO);
        // 其餘欄位仍完整保留。
        assert_eq!(dto.closing_price, dec!(955.5));
    }

    /// 驗證 TPEx 位置式解析：正常列成功、含自帶正負號的漲跌。
    #[test]
    fn from_with_exchange_tpex_parses_valid_row() {
        let row = vec![
            "5483".to_string(),      // 0: 代號
            "中美晶".to_string(),    // 1: 名稱
            "100.00".to_string(),    // 2: 收盤價
            "-1.50".to_string(),     // 3: 漲跌（自帶負號）
            "98.50".to_string(),     // 4: 開盤價
            "101.00".to_string(),    // 5: 最高價
            "98.00".to_string(),     // 6: 最低價
            "10,000".to_string(),    // 7: 成交股數
            "1,000,000".to_string(), // 8: 成交金額
            "500".to_string(),       // 9: 成交筆數
            "100.00".to_string(),    // 10: 最後買價
            "10".to_string(),        // 11: 最後買量
            "100.50".to_string(),    // 12: 最後賣價
            "20".to_string(),        // 13: 最後賣量
        ];
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let dto = DailyQuoteDto::from_with_exchange(StockExchange::TPEx, &row, date).unwrap();

        assert_eq!(dto.symbol, "5483");
        assert_eq!(dto.closing_price, dec!(100.00));
        assert_eq!(dto.change, dec!(-1.50));
        assert_eq!(dto.trading_volume, dec!(10000));
    }

    /// 驗證 TPEx 資料列長度不足（缺欄位）時整列被拒絕。
    #[test]
    fn from_with_exchange_tpex_rejects_short_row() {
        let row = vec!["5483".to_string(), "中美晶".to_string()];
        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

        let err = DailyQuoteDto::from_with_exchange(StockExchange::TPEx, &row, date).unwrap_err();

        assert!(matches!(err, QuoteParseError::MissingField { .. }));
    }
}
