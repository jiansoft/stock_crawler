use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;

/// 大盤與個人帳戶市值總覽領域實體 (Aggregate Root)。
///
/// 記錄全體帳戶以及各主要帳戶在特定交易日的收盤市值總額與更新時間。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyFlow {
    /// 交易日期
    pub date: NaiveDate,
    /// 建立時間
    pub created_at: DateTime<Local>,
    /// 最後更新時間
    pub updated_at: DateTime<Local>,
    /// Unice 帳戶市值總額
    pub unice: Decimal,
    /// Eddie 帳戶市值總額
    pub eddie: Decimal,
    /// 全帳戶市值總額
    pub sum: Decimal,
}

impl Default for MoneyFlow {
    fn default() -> Self {
        // 設定預設交易日為 epoch
        let ep = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        MoneyFlow {
            date: ep,
            created_at: Local::now(),
            updated_at: Local::now(),
            unice: Decimal::ZERO,
            eddie: Decimal::ZERO,
            sum: Decimal::ZERO,
        }
    }
}

/// 每日持股層級市值明細領域實體。
///
/// 紀錄特定交易日、特定會員持有的單一股票之股數、成本、收盤市值與損益比率等明細數據。
#[derive(Debug, Clone, PartialEq)]
pub struct MoneyFlowDetail {
    /// 交易日期
    pub date: NaiveDate,
    /// 建立時間
    pub created_time: DateTime<Local>,
    /// 最後更新時間
    pub updated_time: DateTime<Local>,
    /// 股票代號
    pub security_code: String,
    /// 持有總股數
    pub total_shares: i64,
    /// 主鍵序號
    pub serial: i64,
    /// 前一交易日市值
    pub previous_day_market_value: f64,
    /// 每股平均成本
    pub average_unit_price_per_share: f64,
    /// 佔該會員當日持股總市值的比例（百分比）
    pub ratio: f64,
    /// 與前一交易日相較的損益變動金額
    pub previous_day_profit_and_loss: f64,
    /// 當日收盤市值
    pub market_value: f64,
    /// 累計持股成本
    pub cost: f64,
    /// 估算證券交易稅
    pub transfer_tax: f64,
    /// 當日累計損益金額
    pub profit_and_loss: f64,
    /// 當日累計損益百分比
    pub profit_and_loss_percentage: f64,
    /// 相對前一交易日的損益變動百分比
    pub previous_day_profit_and_loss_percentage: f64,
    /// 當日收盤價
    pub closing_price: f64,
    /// 會員識別碼（0 代表全體聚合）
    pub member_id: i32,
}

impl Default for MoneyFlowDetail {
    fn default() -> Self {
        let ep = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        MoneyFlowDetail {
            date: ep,
            created_time: Local::now(),
            updated_time: Local::now(),
            security_code: String::new(),
            total_shares: 0,
            serial: 0,
            previous_day_market_value: 0.0,
            average_unit_price_per_share: 0.0,
            ratio: 0.0,
            previous_day_profit_and_loss: 0.0,
            market_value: 0.0,
            cost: 0.0,
            transfer_tax: 0.0,
            profit_and_loss: 0.0,
            profit_and_loss_percentage: 0.0,
            previous_day_profit_and_loss_percentage: 0.0,
            closing_price: 0.0,
            member_id: 0,
        }
    }
}

/// 每日交易批次層級市值明細領域實體。
///
/// 紀錄特定交易日、特定會員名下單一股票之各別買入批次的股數、單批成本、收盤市值與損益明細。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyFlowDetailMore {
    /// 主鍵序號
    pub serial: i64,
    /// 會員識別碼
    pub member_id: i64,
    /// 統計日期
    pub date: NaiveDate,
    /// 原始買入交易日期
    pub transaction_date: NaiveDate,
    /// 股票代號
    pub security_code: String,
    /// 當日收盤價
    pub closing_price: Decimal,
    /// 此批次持有股數
    pub number_of_shares_held: i64,
    /// 此批次每股買入成本
    pub unit_price_per_share: Decimal,
    /// 此批次總買入成本
    pub cost: Decimal,
    /// 此批次當日收盤市值
    pub market_value: Decimal,
    /// 此批次當日損益金額
    pub profit_and_loss: Decimal,
    /// 此批次當日損益百分比
    pub profit_and_loss_percentage: Decimal,
    /// 建立時間
    pub created_time: DateTime<Local>,
    /// 最後更新時間
    pub updated_time: DateTime<Local>,
}

impl Default for MoneyFlowDetailMore {
    fn default() -> Self {
        let ep = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        MoneyFlowDetailMore {
            serial: 0,
            member_id: 0,
            date: ep,
            transaction_date: ep,
            security_code: String::new(),
            closing_price: Decimal::ZERO,
            number_of_shares_held: 0,
            unit_price_per_share: Decimal::ZERO,
            cost: Decimal::ZERO,
            market_value: Decimal::ZERO,
            profit_and_loss: Decimal::ZERO,
            profit_and_loss_percentage: Decimal::ZERO,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }
}

/// 每日會員市值垂直總覽領域實體。
///
/// 用於支援未來無限擴充多會員結構的垂直表格表示法，記錄特定交易日與特定會員的市值總額。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyFlowMember {
    /// 交易日期
    pub date: NaiveDate,
    /// 會員編號；0 代表全體總合
    pub member_id: i64,
    /// 當日收盤市值總額
    pub market_value: Decimal,
    /// 建立時間
    pub created_at: DateTime<Local>,
    /// 最後更新時間
    pub updated_at: DateTime<Local>,
}

impl Default for MoneyFlowMember {
    fn default() -> Self {
        let ep = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        MoneyFlowMember {
            date: ep,
            member_id: 0,
            market_value: Decimal::ZERO,
            created_at: Local::now(),
            updated_at: Local::now(),
        }
    }
}

/// 會員當日與前一交易日市值對照領域實體。
///
/// 封裝會員在當前交易日與前一交易日的市值，以便發送通知時快速比較市值增減幅度。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyFlowMemberWithPreviousDay {
    /// 當日資料日期
    pub date: NaiveDate,
    /// 前一個交易日日期
    pub previous_date: Option<NaiveDate>,
    /// 會員編號；0 代表全體總合
    pub member_id: i64,
    /// 當日收盤市值總額
    pub market_value: Decimal,
    /// 前一交易日收盤市值總額
    pub previous_market_value: Decimal,
}

impl Default for MoneyFlowMemberWithPreviousDay {
    fn default() -> Self {
        let ep = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        MoneyFlowMemberWithPreviousDay {
            date: ep,
            previous_date: None,
            member_id: 0,
            market_value: Decimal::ZERO,
            previous_market_value: Decimal::ZERO,
        }
    }
}

impl MoneyFlowDetail {
    /// 判定此股票持有部位目前是否處於獲利狀態。
    pub fn is_profitable(&self) -> bool {
        // 損益大於零代表獲利
        self.profit_and_loss > 0.0
    }
}

impl MoneyFlowDetailMore {
    /// 判定此交易批次目前是否處於獲利狀態。
    pub fn is_profitable(&self) -> bool {
        // 損益大於零代表獲利
        self.profit_and_loss > Decimal::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_money_flow_detail_is_profitable() {
        // 損益為正
        let detail_profit = MoneyFlowDetail {
            profit_and_loss: 1500.0,
            ..Default::default()
        };
        assert!(detail_profit.is_profitable());

        // 損益為負
        let detail_loss = MoneyFlowDetail {
            profit_and_loss: -500.0,
            ..Default::default()
        };
        assert!(!detail_loss.is_profitable());

        // 損益為零
        let detail_zero = MoneyFlowDetail {
            profit_and_loss: 0.0,
            ..Default::default()
        };
        assert!(!detail_zero.is_profitable());
    }

    #[test]
    fn test_money_flow_detail_more_is_profitable() {
        // 損益為正
        let detail_profit = MoneyFlowDetailMore {
            profit_and_loss: dec!(3000),
            ..Default::default()
        };
        assert!(detail_profit.is_profitable());

        // 損益為負
        let detail_loss = MoneyFlowDetailMore {
            profit_and_loss: dec!(-200),
            ..Default::default()
        };
        assert!(!detail_loss.is_profitable());
    }
}
