//! `/market/movers` 的 request、response 與 OpenAPI schema（當日漲跌幅／成交量排行）。
//!
//! 所有缺值欄位皆保留 `Option`，讓 serde 輸出 JSON `null` 而非猜測成零值。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// OpenAPI 文件使用的漲跌幅／成交量排行排序鍵（movers 計畫 §4.1）。
///
/// 刻意不提供「成交金額排行」：即時報價快照沒有成交金額欄位，若提供此排序
/// 鍵，同一個參數在盤中與收盤後會有不同語意（盤中無法計算、收盤後才有），
/// 對呼叫端（通常是 LLM）而言比「不提供」更危險。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
// 前者：此 enum 僅提供 OpenAPI schema，程式不會建立它的實例。
// 後者：變體共同的 `Top` 前綴是對外契約的一部分（`top_gainers` 等字面值直接
// 由變體名稱轉成 snake_case），不可為了消除 clippy 的命名提示而改名。
#[allow(dead_code, clippy::enum_variant_names)]
pub(crate) enum MoversRankByValue {
    /// 漲幅由高到低。
    TopGainers,
    /// 跌幅由深到淺（漲跌幅由低到高）。
    TopLosers,
    /// 成交量由大到小。
    TopVolume,
}

/// 當日漲跌幅／成交量排行中的單一股票（movers 計畫 §4.4）。
///
/// 同一個結構同時服務兩種資料來源，缺的欄位一律是 `null`，**不可用 `0` 代替**：
///
/// - 盤中（`source = "realtime"`）：資料來自第三方網站採集的即時報價快照，
///   有 `last_close`、`volume_lots`（張）與 `source_site`；快照沒有成交金額
///   與成交筆數，因此 `volume_shares`、`trade_value`、`transaction` 為 `null`。
/// - 收盤後（`source = "closing"`）：資料來自 `"DailyQuotes"`，有
///   `volume_shares`（股）、`trade_value`、`transaction`；該表沒有昨收欄位，
///   因此 `last_close` 為 `null`，`volume_lots` 與 `source_site` 亦為 `null`。
///
/// 成交量刻意不做張／股換算：一張雖然通常是 1,000 股，但零股交易會讓換算
/// 結果失真，寧可用 `null` 誠實表示「這個來源沒有這個單位的數字」。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct MarketMover {
    /// 一起始的名次。
    pub(crate) rank: u32,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場編號（上市 2、上櫃 4）。
    pub(crate) market_id: i32,
    /// 產業分類編號。
    pub(crate) industry_id: i32,
    /// 成交價：盤中為最新成交價，收盤後為收盤價。
    pub(crate) price: Option<f64>,
    /// 漲跌（元）。
    pub(crate) change: Option<f64>,
    /// 漲跌幅（%）。
    pub(crate) change_percent: Option<f64>,
    /// 開盤價。
    pub(crate) open: Option<f64>,
    /// 最高價。
    pub(crate) high: Option<f64>,
    /// 最低價。
    pub(crate) low: Option<f64>,
    /// 昨收價；收盤來源沒有此欄位，固定 `null`。
    pub(crate) last_close: Option<f64>,
    /// 成交量（張）；僅即時來源有值。
    pub(crate) volume_lots: Option<f64>,
    /// 成交量（股）；僅收盤來源有值。
    pub(crate) volume_shares: Option<f64>,
    /// 成交金額（元）；僅收盤來源有值。
    pub(crate) trade_value: Option<f64>,
    /// 成交筆數；僅收盤來源有值。
    pub(crate) transaction: Option<f64>,
    /// 即時報價的採集站點（例如 `HiStock`、`Yahoo`）；僅即時來源有值。
    pub(crate) source_site: Option<String>,
}

/// 當日漲跌幅／成交量排行的成功回應（movers 計畫 §4.3）。
///
/// 回應形狀不隨資料來源改變；呼叫端必須讀 `source` 與 `is_realtime` 才知道
/// 手上這份資料是盤中即時還是某一日的收盤結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MarketMoversResponse {
    /// 資料日期（`YYYY-MM-DD`）：即時來源為台北時區今日；收盤來源為該批
    /// `"DailyQuotes"` 的實際交易日。
    ///
    /// **13:30 收盤到 15:00 收盤排程之間**，即時快取已清空但當日日線尚未
    /// 寫入，此時這個欄位會是**前一交易日**——呼叫端必須據此提醒使用者，
    /// 不可宣稱為當日行情。
    pub(crate) data_as_of: String,
    /// 資料來源：`realtime`（盤中即時快照）或 `closing`（收盤日線）。
    pub(crate) source: String,
    /// 是否為即時資料；等同於 `source == "realtime"`。
    pub(crate) is_realtime: bool,
    /// 實際採用的排序鍵：`top_gainers`、`top_losers` 或 `top_volume`。
    pub(crate) rank_by: String,
    /// 實際採用的市場條件：`all`、`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 即時快照批次中最新的更新時間（UTC ISO 8601）；收盤來源為 `null`。
    pub(crate) snapshot_updated_at: Option<String>,
    /// 依 `rank_by` 排序、同值以股票代號由小到大穩定排序的排行。
    pub(crate) movers: Vec<MarketMover>,
}

/// 當日漲跌幅／成交量排行 endpoint 的 query string（movers 計畫 §4.1）。
///
/// **刻意不提供「指定資料來源」的參數**：現在是不是盤中，由伺服器端依即時
/// 快取狀態判斷（movers 計畫 §3），呼叫端沒有能力正確判斷，把選擇權交出去
/// 只會製造「拿前一交易日資料當今日行情」這類錯誤答案。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarketMoversParams {
    /// 排序鍵：`top_gainers`（預設）、`top_losers` 或 `top_volume`。
    #[param(value_type = MoversRankByValue, default = "top_gainers")]
    pub(crate) rank_by: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 最多回傳筆數，預設 20，範圍 1–50。
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub(crate) limit: Option<u8>,
}
