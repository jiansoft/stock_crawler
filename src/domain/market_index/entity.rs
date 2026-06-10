use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;

/// 代表市場指數（例如：TAIEX 台灣加權股價指數）的領域實體。
///
/// 包含大盤交易量、交易金額、交易筆數、漲跌與收盤指數等核心數據。
#[derive(Debug, Clone)]
pub struct MarketIndex {
    /// 指數分類代碼，例如 "TAIEX"
    pub category: String,
    /// 指數的交易日期
    pub date: NaiveDate,
    /// 收盤指數數值
    pub index: Decimal,
    /// 相較於前一交易日的漲跌點數
    pub change: Decimal,
    /// 成交金額 (元)
    pub trade_value: Decimal,
    /// 成交筆數
    pub transaction: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    /// 建立時間
    pub create_time: DateTime<Local>,
    /// 最後更新時間
    pub update_time: DateTime<Local>,
}

impl MarketIndex {
    /// 建立全新市場指數實體的工廠方法。
    ///
    /// # 參數
    /// * `category` - 指數分類
    /// * `date` - 指數日期
    /// * `index` - 收盤指數值
    /// * `change` - 漲跌點數
    /// * `trade_value` - 成交金額
    /// * `transaction` - 成交筆數
    /// * `trading_volume` - 成交股數
    pub fn new(
        category: String,
        date: NaiveDate,
        index: Decimal,
        change: Decimal,
        trade_value: Decimal,
        transaction: Decimal,
        trading_volume: Decimal,
    ) -> Self {
        let now = Local::now();
        Self {
            category,
            date,
            index,
            change,
            trade_value,
            transaction,
            trading_volume,
            create_time: now,
            update_time: now,
        }
    }

    /// 從持久化儲存重建 (Reconstitute) 市場指數實體的工廠方法。
    ///
    /// 用於從資料庫還原狀態，不會觸發任何建立時間的重設。
    #[allow(clippy::too_many_arguments)]
    pub fn reconstitute(
        category: String,
        date: NaiveDate,
        index: Decimal,
        change: Decimal,
        trade_value: Decimal,
        transaction: Decimal,
        trading_volume: Decimal,
        create_time: DateTime<Local>,
        update_time: DateTime<Local>,
    ) -> Self {
        Self {
            category,
            date,
            index,
            change,
            trade_value,
            transaction,
            trading_volume,
            create_time,
            update_time,
        }
    }
}

impl crate::core::util::map::Keyable for MarketIndex {
    fn key(&self) -> String {
        format!("{}-{}", self.date, self.category)
    }

    fn key_with_prefix(&self) -> String {
        format!("MarketIndex:{}", self.key())
    }
}
