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

/// 計算漲跌幅（%）：漲跌 ÷ 參考價 × 100，參考價為「收盤 − 漲跌」。
///
/// 交易所與券商的漲跌幅都以當日**參考價**為基準，而參考價不一定等於前一個成交日的收盤：
/// 除權息、減資恢復交易，或前幾天沒有成交、參考價依收盤揭示的買賣價調整時都會不同。
/// 交易所給的漲跌就是相對參考價，所以「收盤 − 漲跌」就是參考價，漲跌幅的正負號也一定與漲跌一致。
///
/// 過去改用快取的前一個成交日收盤計算，在這些日子會算錯：6661 威健生技 2026-10-05 收 16.60、
/// 漲跌 -0.15（參考價 16.75），以前一個成交日收盤 16.20 計算得到 +2.47%，正負號與漲跌相反；
/// 2026 年約 15% 的日報價因此與官方不同。
///
/// 漲跌為 0（含除息日交易所標示「除息」）或推不出正的參考價時為 0。
pub fn change_range_percent(closing_price: Decimal, change: Decimal) -> Decimal {
    if change.is_zero() {
        return Decimal::ZERO;
    }
    let reference = closing_price - change;
    if reference <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    change / reference * Decimal::ONE_HUNDRED
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod change_range_tests;
