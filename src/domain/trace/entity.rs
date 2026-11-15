use rust_decimal::Decimal;

/// 代表個股價格追蹤（警示區間）的領域實體。
///
/// 當個股價格低於 `floor`（下限）或高於 `ceiling`（上限）時，系統將觸發警示通知。
/// 每位會員各有一份清單：同一檔股票可以被多位會員以各自的上下限追蹤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceTrace {
    /// 設定這筆追蹤條件的會員編號
    pub member_id: i64,
    /// 追蹤的股票代號，例如 "2330"
    pub stock_symbol: String,
    /// 監控下限價格
    pub floor: Decimal,
    /// 監控上限價格
    pub ceiling: Decimal,
}

impl PriceTrace {
    /// 建立全新價格追蹤實體的工廠方法。
    ///
    /// # 參數
    /// * `member_id` - 設定這筆條件的會員編號
    /// * `stock_symbol` - 股票代號
    /// * `floor` - 下限價
    /// * `ceiling` - 上限價
    pub fn new(member_id: i64, stock_symbol: String, floor: Decimal, ceiling: Decimal) -> Self {
        Self {
            member_id,
            stock_symbol,
            floor,
            ceiling,
        }
    }
}

impl crate::core::util::map::Keyable for PriceTrace {
    /// 產生用於快取或識別的唯一鍵值；含會員編號，甲的通知去重不會擋掉乙的通知。
    fn key(&self) -> String {
        format!(
            "{}-{}-{}-{}",
            self.member_id, self.stock_symbol, self.floor, self.ceiling
        )
    }

    /// 產生帶有類型前綴的鍵值。
    fn key_with_prefix(&self) -> String {
        format!("Trace:{}", self.key())
    }
}
