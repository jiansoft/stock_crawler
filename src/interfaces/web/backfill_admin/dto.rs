use serde::{Deserialize, Serialize};

use super::state::BackfillJob;

/// 收盤彙總手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct ClosingAggregateRequest {
    /// 交易日期，格式必須為 `YYYY-MM-DD`。
    pub(super) date: String,
}

/// 台股加權指數手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct TaiwanStockIndexRequest {
    /// 回補目標日期，格式必須為 `YYYY-MM-DD`。
    ///
    /// TWSE API 會依此日期回傳該月份所有交易日的指數資料。
    pub(super) date: String,
}

/// 各股每日收盤報價手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct DailyQuotesRequest {
    /// 交易日期，格式必須為 `YYYY-MM-DD`。
    pub(super) date: String,
}

/// 年度類手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct YearRequest {
    /// 回補目標年度，例如 `2026`。
    pub(super) year: i32,
}

/// 證券代號類手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct SecurityCodeRequest {
    /// 股票、ETF 或其他證券代號。
    pub(super) security_code: String,
}

/// 歷史日報價回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct QuoteHistoryRequest {
    /// 股票代號，以逗號、空白或換行分隔；留空表示全部未下市的 `00` 開頭 ETF／ETN。
    #[serde(default)]
    pub(super) stock_symbols: String,
    /// 起始月份，格式 `YYYY-MM`（含）。
    pub(super) from: String,
    /// 結束月份，格式 `YYYY-MM`（含）。
    pub(super) to: String,
}

/// CAGR 重算的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct CagrRequest {
    /// 基準日，格式 `YYYY-MM-DD`；留空表示採用資料庫中最新的交易日。
    #[serde(default)]
    pub(super) date: String,
}

/// 單一統計期間歷史回填的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct CagrPeriodRequest {
    /// 期間代碼，例如 `Y7`。
    pub(super) period: String,
}

/// 公司行動登錄的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct CorporateActionRequest {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 生效日（換發後恢復交易的第一個交易日），格式 `YYYY-MM-DD`。
    pub(super) effective_date: String,
    /// 股數變動比例：持有 1 股在事件後變成幾股。
    pub(super) share_ratio: String,
    /// 備註。
    #[serde(default)]
    pub(super) note: String,
}

/// 公司行動登錄成功時的 HTTP response body。
#[derive(Debug, Serialize)]
pub(super) struct CorporateActionResponse {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 該股目前已登錄的所有公司行動（生效日與比例）。
    pub(super) actions: Vec<CorporateActionItem>,
}

/// 已登錄的單筆公司行動。
#[derive(Debug, Serialize)]
pub(super) struct CorporateActionItem {
    /// 生效日。
    pub(super) effective_date: String,
    /// 股數變動比例。
    pub(super) share_ratio: String,
    /// 備註。
    pub(super) note: String,
}

/// 建立 job 成功時的 HTTP response body。
#[derive(Debug, Serialize)]
pub(super) struct StartJobResponse {
    /// 已建立的 job。
    pub(super) job: BackfillJob,
}

/// API 錯誤回應。
#[derive(Debug, Serialize)]
pub(super) struct ErrorResponse {
    /// 可讀的錯誤原因。
    pub(super) error: String,
}

/// Manual backfill 單頁操作介面（內容在同目錄的 `index.html`）。
pub(super) const INDEX_HTML: &str = include_str!("index.html");
