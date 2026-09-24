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

    /// <summary>
    /// 當每日帳戶市值重新計算與儲存完成時觸發。
    /// </summary>
    MoneyFlowRecalculated {
        /// 市值重算的基準日期
        date: chrono::NaiveDate,
        /// 事件發生時間
        occurred_at: DateTime<Local>,
    },

    /// <summary>
    /// 當每日除權息提醒事件觸發時觸發。
    /// </summary>
    ExDividendReminderTriggered {
        /// 提醒基準日期 (今日交易日)
        date: chrono::NaiveDate,
        /// 下一個交易日日期
        next_trading_date: chrono::NaiveDate,
        /// 事件發生時間
        occurred_at: DateTime<Local>,
    },

    /// <summary>
    /// 當一個月份的台股月營收全部更新完畢時觸發。
    /// </summary>
    ///
    /// 事件只帶月份，實際要通知哪些股票由 handler 依「目前持股」自行查詢；
    /// 逐檔發事件會讓一次更新產生上千個事件，也無從彙總成一則訊息。
    MonthlyRevenueUpdated {
        /// 營收月份 (yyyyMM)
        date: i64,
        /// 事件發生時間
        occurred_at: DateTime<Local>,
    },

    /// <summary>
    /// 當一個季度的台股財報更新完畢（含 ROE／ROA 補值）時觸發。
    /// </summary>
    QuarterlyFinancialsUpdated {
        /// 財報年度
        year: i32,
        /// 財報季度 (Q1, Q2, Q3, Q4)
        quarter: String,
        /// 事件發生時間
        occurred_at: DateTime<Local>,
    },
}
