//! 長生命週期主快取。
//!
//! 此模組負責維護 crawler 執行期間會反覆共用的主資料快取，
//! 包含股票主檔、最後交易日行情、歷史高低統計、月營收索引、
//! 即時報價快照，以及產業與交易市場對照資訊。

use std::{collections::HashMap, sync::RwLock};

use once_cell::sync::Lazy;

use super::{
    lookup::{default_exchange_markets, default_industries},
    realtime::RealtimeSnapshot,
};
use crate::domain::market_index::MarketIndex;
use crate::infra::database::table::{last_daily_quotes, revenue, stock_exchange_market};

/// 全域共享資料快取實例。
///
/// 這是整個 crawler 程式在執行期間共用的主快取容器。
/// 請在服務啟動時先呼叫 [`Share::load`] 完成初始化，再進行讀取。
pub static SHARE: Lazy<Share> = Lazy::new(Default::default);

/// 各類長生命週期快取的集中管理器。
///
/// 主要用途：
/// - 讓不同 crawler / backfill / event 流程共用同一份資料，降低重複查詢成本。
/// - 提供具型別的快取查詢與更新入口，避免散落在各模組中直接操作 `HashMap`。
/// - 在讀鎖失敗時以安全預設值降級，降低流程中斷風險。
///
/// 注意：
/// - `new()` 只建立空容器與靜態對照表，不會觸發資料庫或網路 I/O。
/// - 真正載入資料需呼叫 [`Self::load`]。
pub struct Share {
    /// 存放台股歷年指數
    pub(super) indices: RwLock<HashMap<String, MarketIndex>>,
    /// 存放台股股票代碼
    pub stocks: RwLock<HashMap<String, crate::domain::registry::entity::Stock>>,
    /// 月營收的快取(防止重複寫入)，第一層 Key:日期 yyyyMM 第二層 Key:股號
    pub(super) last_revenues: RwLock<HashMap<i64, HashMap<String, revenue::Revenue>>>,
    /// 存放最後交易日股票報價數據
    pub(super) last_trading_day_quotes:
        RwLock<HashMap<String, last_daily_quotes::LastDailyQuotes>>,
    /// 股票歷史、淨值比等最高與最低數據。
    ///
    /// 啟動時會先從資料庫載入；若後續抓到更新資料，應同步更新資料庫與這份快取。
    pub quote_history_records:
        RwLock<HashMap<String, crate::domain::quote::entity::QuoteHistoryRecord>>,
    /// 股票產業分類
    pub(super) industries: HashMap<String, i32>,
    /// 股票產業分類(2, 'TAI', '上市', 1),(4, 'TWO', '上櫃', 2), (5, 'TWE', '興櫃', 2);
    pub(super) exchange_markets: HashMap<i32, stock_exchange_market::StockExchangeMarket>,
    /// 目前的 IP
    pub(super) current_ip: RwLock<String>,
    /// 股票即時報價快照快取 (目前主要由 HiStock 驅動)
    pub stock_snapshots: RwLock<HashMap<String, RealtimeSnapshot>>,
}

impl Share {
    /// 建立一個新的 `Share` 實例。
    ///
    /// 此方法會初始化：
    /// - 可變快取容器（空的 `HashMap` + `RwLock`）
    /// - 交易市場對照表（上市/上櫃/興櫃）
    /// - 產業名稱與代碼對照表（含部分別名）
    ///
    /// 此方法不會讀取資料庫，也不會發出 HTTP 請求。
    pub fn new() -> Self {
        Self {
            indices: RwLock::new(HashMap::new()),
            stocks: RwLock::new(HashMap::new()),
            last_revenues: RwLock::new(HashMap::new()),
            last_trading_day_quotes: RwLock::new(HashMap::new()),
            quote_history_records: RwLock::new(HashMap::new()),
            industries: default_industries(),
            exchange_markets: default_exchange_markets(),
            current_ip: RwLock::new(String::new()),
            stock_snapshots: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for Share {
    fn default() -> Self {
        Self::new()
    }
}
