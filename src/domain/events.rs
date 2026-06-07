use chrono::{DateTime, Local};
use rust_decimal::Decimal;

/// <summary>
/// 表示領域內發生的重要事件 (Domain Event)。
/// 所有事件皆為唯讀且不可變，代表已發生的事實。
/// </summary>
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    /// <summary>
    /// 當新的證券主檔被成功註冊時觸發。
    /// </summary>
    StockRegistered {
        /// 證券代碼 (如 "2330")
        symbol: String,
        /// 證券名稱 (如 "台積電")
        name: String,
        /// 交易所市場識別代碼
        market_id: i32,
        /// 產業分類識別代碼
        industry_id: i32,
        /// 事件發生時間
        occurred_at: DateTime<Local>,
    },

    /// <summary>
    /// 當既有證券的身份識別資訊 (名稱、市場、產業) 發生變更時觸發。
    /// </summary>
    StockIdentityChanged {
        /// 證券代碼
        symbol: String,
        /// 變更前的舊名稱
        old_name: String,
        /// 變更後的新名稱
        new_name: String,
        /// 變更前的舊市場代碼
        old_market_id: i32,
        /// 變更後的新市場代碼
        new_market_id: i32,
        /// 變更前的舊產業代碼
        old_industry_id: i32,
        /// 變更後的新產業代碼
        new_industry_id: i32,
        /// 事件發生時間
        occurred_at: DateTime<Local>,
    },

    /// <summary>
    /// 當證券的每股淨值 (Net Asset Value per share) 發生更新時觸發。
    /// </summary>
    NetAssetValueUpdated {
        /// 證券代碼
        symbol: String,
        /// 變更前的舊淨值
        old_nav: Decimal,
        /// 變更後的新淨值
        new_nav: Decimal,
        /// 事件發生時間
        occurred_at: DateTime<Local>,
    },

    /// <summary>
    /// 當大盤指數更新時觸發。
    /// </summary>
    StockIndexUpdated {
        /// 指數日期
        date: chrono::NaiveDate,
        /// 收盤指數值
        index: Decimal,
        /// 漲跌點數
        change: Decimal,
        /// 事件發生時間
        occurred_at: DateTime<Local>,
    },
}
