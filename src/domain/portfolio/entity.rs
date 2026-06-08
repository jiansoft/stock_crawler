use chrono::{DateTime, Local};
use rust_decimal::Decimal;

/// 持股明細領域實體 (Domain Entity)。
///
/// 代表會員對特定股票的單筆購入與庫存明細，負責累積領取的股利狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockOwnershipDetail {
    /// 序號
    pub serial: i64,
    /// 股票代號
    pub security_code: String,
    /// 會員編號
    pub member_id: i64,
    /// 持有股數
    pub share_quantity: i64,
    /// 買入時平均每股成本
    pub share_price_average: Decimal,
    /// 目前每股成本 (考量折減/調整)
    pub current_cost_per_share: Decimal,
    /// 買入成本總額
    pub holding_cost: Decimal,
    /// 是否已售出
    pub is_sold: bool,
    /// 累積已領取的現金股利 (元)
    pub cumulate_dividends_cash: Decimal,
    /// 累積已領取的股票股利 (股)
    pub cumulate_dividends_stock: Decimal,
    /// 累積已領取的股票股利價值 (元)
    pub cumulate_dividends_stock_money: Decimal,
    /// 累積已領取的總股利價值 (元)
    pub cumulate_dividends_total: Decimal,
    /// 建立時間 (持股入帳時間)
    pub created_time: DateTime<Local>,
}

impl StockOwnershipDetail {
    /// 建立全新的持股明細實體。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        serial: i64,
        security_code: String,
        member_id: i64,
        share_quantity: i64,
        share_price_average: Decimal,
        current_cost_per_share: Decimal,
        holding_cost: Decimal,
        is_sold: bool,
        created_time: DateTime<Local>,
    ) -> Self {
        Self {
            serial,
            security_code,
            member_id,
            share_quantity,
            share_price_average,
            current_cost_per_share,
            holding_cost,
            is_sold,
            cumulate_dividends_cash: Decimal::ZERO,
            cumulate_dividends_stock: Decimal::ZERO,
            cumulate_dividends_stock_money: Decimal::ZERO,
            cumulate_dividends_total: Decimal::ZERO,
            created_time,
        }
    }

    /// 從持久化儲存還原持股明細實體 (Reconstitution)。
    #[allow(clippy::too_many_arguments)]
    pub fn reconstitute(
        serial: i64,
        security_code: String,
        member_id: i64,
        share_quantity: i64,
        share_price_average: Decimal,
        current_cost_per_share: Decimal,
        holding_cost: Decimal,
        is_sold: bool,
        cumulate_dividends_cash: Decimal,
        cumulate_dividends_stock: Decimal,
        cumulate_dividends_stock_money: Decimal,
        cumulate_dividends_total: Decimal,
        created_time: DateTime<Local>,
    ) -> Self {
        Self {
            serial,
            security_code,
            member_id,
            share_quantity,
            share_price_average,
            current_cost_per_share,
            holding_cost,
            is_sold,
            cumulate_dividends_cash,
            cumulate_dividends_stock,
            cumulate_dividends_stock_money,
            cumulate_dividends_total,
            created_time,
        }
    }
}

impl Default for StockOwnershipDetail {
    fn default() -> Self {
        Self::reconstitute(
            0,
            "".to_string(),
            0,
            0,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            false,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Local::now(),
        )
    }
}

impl StockOwnershipDetail {
    /// 更新累積已領取股利狀態。
    pub fn update_cumulate_dividends(
        &mut self,
        cash: Decimal,
        stock: Decimal,
        stock_money: Decimal,
    ) {
        self.cumulate_dividends_cash = cash;
        self.cumulate_dividends_stock = stock;
        self.cumulate_dividends_stock_money = stock_money;
        self.cumulate_dividends_total = cash + stock_money;
    }
}

/// 持股年度已領股利總計之領域實體 (ReceivedDividend)。
///
/// 封裝特定庫存明細在某一發放年度內，所累積領取的所有股利總數。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedDividend {
    /// 序號
    pub serial: i64,
    /// 持股明細序號
    pub stock_ownership_details_serial: i64,
    /// 領取年度
    pub year: i32,
    /// 現金股利 (元)
    pub cash: Decimal,
    /// 股票股利 (股)
    pub stock: Decimal,
    /// 股票股利價值 (元)
    pub stock_money: Decimal,
    /// 合計股利 (元)
    pub total: Decimal,
    /// 建立時間
    pub created_time: DateTime<Local>,
    /// 最後更新時間
    pub updated_time: DateTime<Local>,
}

/// 持股單筆股利發放項目明細之領域實體 (ReceivedDividendItem)。
///
/// 記錄持股明細對應至某次特定的除權息宣告 (Dividend) 所實際領取的明細數值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedDividendItem {
    /// 序號
    pub serial: i64,
    /// 持股明細序號
    pub stock_ownership_details_serial: i64,
    /// 年度總計表序號
    pub dividend_record_detail_serial: i64,
    /// 股利發放宣告序號 (外鍵參考 `dividend.serial`)
    pub dividend_serial: i64,
    /// 現金股利 (元)
    pub cash: Decimal,
    /// 股票股利 (股)
    pub stock: Decimal,
    /// 股票股利價值 (元)
    pub stock_money: Decimal,
    /// 合計股利 (元)
    pub total: Decimal,
    /// 建立時間
    pub created_time: DateTime<Local>,
    /// 最後更新時間
    pub updated_time: DateTime<Local>,
}
