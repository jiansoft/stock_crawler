//! Data API 的 request、response 與 OpenAPI schema。
//!
//! 此模組刻意只放 HTTP 契約型別，避免 SQLx 的資料庫列型別滲漏到 API；所有
//! 缺值欄位皆保留 `Option`，讓 serde 輸出 JSON `null` 而非猜測成零值。
//!
//! 型別依 endpoint 分組放在 `stocks`、`fundamentals`、`screening`、`market`
//! 與 `cagr` 五個子模組，並在此重新導出；跨組共用的市場參數列舉與通用回應
//! 留在本檔。

mod cagr;
mod fundamentals;
mod market;
mod screening;
mod stocks;

use serde::Serialize;
use utoipa::ToSchema;

pub(super) use cagr::{
    CagrCoverageInfo, CagrPeriodItem, CagrRankingItem, CagrRankingParams, CagrRankingResponse,
    CagrSummary, CagrSymbolParams, CagrSymbolResponse,
};
pub(super) use fundamentals::{
    Dividend, DividendHistoryParams, DividendHistoryResponse, FinancialStatement,
    FinancialStatementHistoryResponse, MonthlyRevenue, MonthlyRevenueResponse,
    RevenueHistoryParams, StatementHistoryParams, StockValuation, StockValuationResponse,
    ValuationParams,
};
pub(super) use market::{
    DividendCalendarEvent, DividendCalendarParams, DividendCalendarResponse, DividendYieldRank,
    DividendYieldRankingParams, DividendYieldRankingResponse, MarketBreadth, MarketBreadthParams,
    MarketBreadthResponse, MarketIndexHistoryParams, MarketIndexHistoryResponse, MarketIndexPoint,
    QfiiHolding, QfiiHoldingRankingParams, QfiiHoldingRankingResponse,
};
pub(super) use screening::{ScreenedStock, StockScreeningParams, StockScreeningResponse};
pub(super) use stocks::{
    DailyQuote, HistoricalQuote, HistoryParams, LatestQuoteResponse, PriceHistoryResponse,
    QuoteHistoryRecord, RealtimeSnapshotResponse, SearchParams, SearchResponse, Stock,
    StockProfile,
};

/// OpenAPI 文件使用的三種查詢市場值。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
enum MarketParamValue {
    /// 上市與上櫃合併。
    All,
    /// 僅上市。
    Twse,
    /// 僅上櫃。
    Tpex,
}

/// 統一錯誤回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ErrorBody {
    /// 不含內部實作細節的錯誤訊息。
    pub(crate) error: String,
}

/// 健康檢查成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct HealthResponse {
    /// 服務狀態。
    pub(crate) status: &'static str,
}
