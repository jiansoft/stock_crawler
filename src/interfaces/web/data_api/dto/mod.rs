//! Data API 的 request、response 與 OpenAPI schema。
//!
//! 此模組刻意只放 HTTP 契約型別，避免 SQLx 的資料庫列型別滲漏到 API；所有
//! 缺值欄位皆保留 `Option`，讓 serde 輸出 JSON `null` 而非猜測成零值。
//!
//! 型別依 endpoint 分組放在 `stocks`、`fundamentals`、`dividend_fill`、`financial_reports`、
//! `financial_health`、`chip`、`institutional_flow`、`margin`、`market_regime`、`market_valuation`、`screening`、`market`、`industries`、`portfolio`、`portfolio_chip`、`portfolio_dividend_fill`、`portfolio_history`、`dividend_forecast`、`cagr`、`valuation_band`、`peers`、`technicals`、`signals`、`signal_history` 與 `relative_strength` 等子模組，並在此重新導出；跨組共用的市場參數列舉與通用回應
//! 留在本檔。

mod cagr;
mod chip;
mod dividend_fill;
mod dividend_forecast;
mod financial_health;
mod financial_reports;
mod fundamentals;
mod industries;
mod institutional_flow;
mod margin;
mod market;
mod market_regime;
mod market_valuation;
mod peers;
mod portfolio;
mod portfolio_chip;
mod portfolio_dividend_fill;
mod portfolio_history;
mod relative_strength;
mod screen_backtest;
mod screening;
mod signal_history;
mod signals;
mod stocks;
mod technicals;
mod timeline;
mod valuation_band;

use serde::Serialize;
use utoipa::ToSchema;

pub(super) use cagr::{
    CagrCoverageInfo, CagrMetricParamValue, CagrPeriodItem, CagrPeriodParamValue, CagrRankingItem,
    CagrRankingParams, CagrRankingResponse, CagrSummary, CagrSymbolParams, CagrSymbolResponse,
};
pub(super) use chip::{
    BrokerFlowDay, BrokerNet, ChipDay, ChipParams, ChipResponse, ChipStreak, HolderWeek,
    InsiderSummary,
};
pub(super) use dividend_fill::{
    DividendFillEvent, DividendFillParams, DividendFillResponse, DividendFillSummary,
};
pub(super) use dividend_forecast::{
    DividendForecastItem, DividendForecastMonth, DividendForecastParams, DividendForecastResponse,
    ForecastBasisValue,
};
pub(super) use financial_health::{
    FinancialHealthParams, FinancialHealthResponse, HealthCategoryValue, HealthCriterion,
    HealthValueUnitValue,
};
pub(super) use financial_reports::{
    AMOUNT_UNIT, BalanceSheetItem, CashFlowStatementItem, FinancialReportParams,
    FinancialReportResponse, IncomeStatementItem, ReportPeriodTypeValue,
};
pub(super) use fundamentals::{
    Dividend, DividendHistoryParams, DividendHistoryResponse, FinancialStatement,
    FinancialStatementHistoryResponse, MonthlyRevenue, MonthlyRevenueResponse,
    RevenueHistoryParams, StatementHistoryParams, StatementPeriodTypeValue, StockValuation,
    StockValuationResponse, ValuationParams,
};
pub(super) use industries::{
    IndustryMover, IndustryPerformance, IndustryPerformanceParams, IndustryPerformanceResponse,
};
pub(super) use institutional_flow::{
    FlowSideParamValue, IndustryFlowItem, IndustryFlowParams, IndustryFlowResponse,
    IndustryFlowStock, InstitutionalFlowItem, InstitutionalFlowParams, InstitutionalFlowResponse,
    InstitutionalStreakPerformanceParams, InstitutionalStreakPerformanceResponse,
    InvestorParamValue,
};
pub(super) use margin::{
    MarginRankByValue, MarginRankItem, MarginRankingParams, MarginRankingResponse,
};
pub(super) use market::{
    CalendarEventTypeValue, DividendCalendarEvent, DividendCalendarParams,
    DividendCalendarResponse, DividendYieldRank, DividendYieldRankingParams,
    DividendYieldRankingResponse, MarketBreadth, MarketBreadthParams, MarketBreadthResponse,
    MarketIndexHistoryParams, MarketIndexHistoryResponse, MarketIndexPoint, MarketMover,
    MarketMoversParams, MarketMoversResponse, MoversRankByValue, QfiiHolding,
    QfiiHoldingRankingParams, QfiiHoldingRankingResponse, QfiiSortValue,
};
pub(super) use market_regime::{
    MarketRegimeBreadth, MarketRegimeParams, MarketRegimeResponse, MarketRegimeValuation,
    MovingAverageCrossItem, MovingAverageStatus,
};
pub(super) use market_valuation::{
    MarketValuationParams, MarketValuationPoint, MarketValuationResponse, ValuationRatioStats,
};
pub(super) use peers::{PeerMetric, PeerStock, PeersParams, PeersResponse};
pub(super) use portfolio::{
    PortfolioHolding, PortfolioIndustry, PortfolioMarginTrendValue, PortfolioSummaryParams,
    PortfolioSummaryResponse, PortfolioWarningValue, WeightedMetricItem, margin_trend_code,
    warning_code,
};
pub(super) use portfolio_chip::{
    PortfolioChipBroker, PortfolioChipBrokerNet, PortfolioChipBrokerRecent, PortfolioChipItem,
    PortfolioChipParams, PortfolioChipResponse,
};
pub(super) use portfolio_dividend_fill::{
    PortfolioDividendFillItem, PortfolioDividendFillParams, PortfolioDividendFillResponse,
};
pub(super) use portfolio_history::{
    PortfolioHistoryParams, PortfolioHistoryResponse, PortfolioMonth,
};
pub(super) use relative_strength::{
    IndustryLeader, IndustryStrengthItem, IndustryStrengthParams, IndustryStrengthResponse,
    RelativeStrengthCrossing, RelativeStrengthHistoryParams, RelativeStrengthHistoryResponse,
    RelativeStrengthItem, RelativeStrengthParams, RelativeStrengthPoint, RelativeStrengthResponse,
};
pub(super) use screen_backtest::{
    BacktestAttributionTotals, BacktestAttributionYear, BacktestContributor, BacktestContributors,
    BacktestCurrent, BacktestExit, BacktestFrontierPoint, BacktestHolding, BacktestIndustryEffect,
    BacktestOffsetDistribution, BacktestOffsetResult, BacktestOffsetYear, BacktestPeriod,
    BacktestStat, BacktestSummary, BacktestWalkForwardYear, BacktestYear,
    ScreenBacktestAttributionResponse, ScreenBacktestFrontierResponse,
    ScreenBacktestOffsetsResponse, ScreenBacktestParams, ScreenBacktestResponse,
    ScreenBacktestWalkForwardResponse, StrategyPresetItem, StrategyPresetsResponse,
    StrategyTrackHolding, StrategyTrackPeriod, StrategyTrackResponse,
};
pub(super) use screening::{
    ScreenedStock, SortOrderParamValue, StockScreenSortValue, StockScreeningParams,
    StockScreeningResponse,
};
pub(super) use signal_history::{
    BenchmarkStat, BenchmarkYear, GroupByParamValue, HorizonStat, PerformanceYear,
    SignalPerformanceItem, SignalPerformanceParams, SignalPerformanceResponse, StockSignalItem,
    StockSignalsParams, StockSignalsResponse,
};
pub(super) use signals::{
    MarketSignalsParams, MarketSignalsResponse, SignalCount, SignalIndicatorValue, SignalKindValue,
    SignalStock,
};
pub(super) use stocks::{
    AdjustedPrices, DailyQuote, HistoricalQuote, HistoryParams, LatestQuoteResponse,
    PriceHistoryResponse, QuoteHistoryRecord, RealtimeSnapshotResponse, SearchParams,
    SearchResponse, Stock, StockProfile,
};
pub(super) use technicals::{
    TechnicalIndicatorsParams, TechnicalIndicatorsResponse, TechnicalPoint, TechnicalSignalItem,
};
pub(super) use timeline::{
    StockTimelineParams, StockTimelineResponse, TimelineEvent, TimelineKindCount, TimelineKindValue,
};
pub(super) use valuation_band::{
    ImpliedPriceBand, RatioBandItem, ValuationBandParams, ValuationBandResponse,
};

/// OpenAPI 文件使用的三種查詢市場值。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum MarketParamValue {
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
