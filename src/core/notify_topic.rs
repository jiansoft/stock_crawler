//! 通知項目目錄（會員系統計畫 7.3）。
//!
//! 系統發出的每一種 Telegram 推送都對應一個 [`Topic`]，收件者可逐項開關。目錄定義在發送方
//! （這裡），服務啟動時寫入資料表 `notify_topic`，stock_go 的設定頁依表產生開關；日後新增一種
//! 通知只要在這裡加一項。發送入口（`core::alert::notify_*`）的第一個參數必須是 [`Topic`]，
//! 沒有指定項目的推送無法編譯。

/// 通知的受眾，決定內容如何產生與分送（7.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Audience {
    /// 系統與資料健康：所有訂閱者內容相同。
    System,
    /// 市場與策略：所有訂閱者內容相同，不含任何人的持股。
    Market,
    /// 持股：依收件者的可見範圍各自產生內容。
    Holdings,
    /// 持股以外的個人資料（價格追蹤）：分送機制與持股相同。
    Personal,
}

impl Audience {
    /// 寫入 `notify_topic.audience` 的值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Market => "market",
            Self::Holdings => "holdings",
            Self::Personal => "personal",
        }
    }
}

/// 設定頁的分組（寫入 `notify_topic.category`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopicCategory {
    /// 除權息與股利。
    Dividend,
    /// 市值與營運回顧。
    Value,
    /// 籌碼。
    Chip,
    /// 基本面與估值。
    Fundamental,
    /// 技術面。
    Technical,
    /// 市場與策略。
    Market,
    /// 價格追蹤。
    PriceTrace,
    /// 系統與資料健康。
    System,
}

impl TopicCategory {
    /// 寫入 `notify_topic.category` 的值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dividend => "dividend",
            Self::Value => "value",
            Self::Chip => "chip",
            Self::Fundamental => "fundamental",
            Self::Technical => "technical",
            Self::Market => "market",
            Self::PriceTrace => "price_trace",
            Self::System => "system",
        }
    }
}

/// 一個通知項目的目錄資料（對應 `notify_topic` 一列）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopicSpec {
    /// 項目代碼，例如 `holdings.payable`。
    pub code: &'static str,
    /// 設定頁顯示的名稱。
    pub name: &'static str,
    /// 何時發送。
    pub description: &'static str,
    /// 設定頁的分組。
    pub category: TopicCategory,
    /// 受眾。
    pub audience: Audience,
    /// 訂閱需要的權限（stock_go 寫入訂閱時檢查）。
    pub required_permission: &'static str,
    /// 只有 root 群組的會員可訂閱（`system.critical`）。
    pub root_only: bool,
    /// 第一次綁定時是否自動訂閱。
    pub default_enabled: bool,
}

/// 通知項目。順序即設定頁的顯示順序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    HoldingsExDividendPreview,
    HoldingsExDividend,
    HoldingsPayable,
    HoldingsMonthlyPayout,
    HoldingsPayableFill,
    HoldingsMarketValue,
    HoldingsRevenue,
    HoldingsFinancialReport,
    HoldingsWeekly,
    HoldingsStrategyOverlap,
    HoldingsBrokerFlow,
    HoldingsChipFlow,
    HoldingsHolderDistribution,
    HoldingsInsiderPledge,
    HoldingsForeignHolding,
    HoldingsEarningsCall,
    HoldingsFinancialHealth,
    HoldingsValuation,
    HoldingsEtfPremium,
    HoldingsTechnical,
    HoldingsBreakout,
    MarketRegime,
    MarketWeekly,
    MarketStrategyRebalance,
    MarketStrategyEdge,
    MarketPublicOffering,
    MarketExDividend,
    PersonalPriceTrace,
    SystemCritical,
    SystemStartup,
    SystemAppUpdate,
    SystemDbHealth,
    SystemHostHealth,
    SystemClosingCheck,
    SystemAfterHours,
    SystemDataHealth,
}

const PORTFOLIO_VIEW: &str = "portfolio.view";
const MARKET_VIEW: &str = "market.view";
const OPS_VIEW: &str = "ops.view";
const PRICE_TRACE_VIEW: &str = "price_trace.view";

const fn holdings(
    code: &'static str,
    name: &'static str,
    description: &'static str,
    category: TopicCategory,
    default_enabled: bool,
) -> TopicSpec {
    TopicSpec {
        code,
        name,
        description,
        category,
        audience: Audience::Holdings,
        required_permission: PORTFOLIO_VIEW,
        root_only: false,
        default_enabled,
    }
}

const fn market(code: &'static str, name: &'static str, description: &'static str) -> TopicSpec {
    TopicSpec {
        code,
        name,
        description,
        category: TopicCategory::Market,
        audience: Audience::Market,
        required_permission: MARKET_VIEW,
        root_only: false,
        default_enabled: false,
    }
}

const fn system(code: &'static str, name: &'static str, description: &'static str) -> TopicSpec {
    TopicSpec {
        code,
        name,
        description,
        category: TopicCategory::System,
        audience: Audience::System,
        required_permission: OPS_VIEW,
        root_only: false,
        default_enabled: true,
    }
}

impl Topic {
    /// 全部通知項目，依設定頁的顯示順序。
    pub const ALL: [Topic; 36] = [
        Self::HoldingsExDividendPreview,
        Self::HoldingsExDividend,
        Self::HoldingsPayable,
        Self::HoldingsMonthlyPayout,
        Self::HoldingsPayableFill,
        Self::HoldingsMarketValue,
        Self::HoldingsRevenue,
        Self::HoldingsFinancialReport,
        Self::HoldingsWeekly,
        Self::HoldingsStrategyOverlap,
        Self::HoldingsBrokerFlow,
        Self::HoldingsChipFlow,
        Self::HoldingsHolderDistribution,
        Self::HoldingsInsiderPledge,
        Self::HoldingsForeignHolding,
        Self::HoldingsEarningsCall,
        Self::HoldingsFinancialHealth,
        Self::HoldingsValuation,
        Self::HoldingsEtfPremium,
        Self::HoldingsTechnical,
        Self::HoldingsBreakout,
        Self::MarketRegime,
        Self::MarketWeekly,
        Self::MarketStrategyRebalance,
        Self::MarketStrategyEdge,
        Self::MarketPublicOffering,
        Self::MarketExDividend,
        Self::PersonalPriceTrace,
        Self::SystemCritical,
        Self::SystemStartup,
        Self::SystemAppUpdate,
        Self::SystemDbHealth,
        Self::SystemHostHealth,
        Self::SystemClosingCheck,
        Self::SystemAfterHours,
        Self::SystemDataHealth,
    ];

    /// 這個項目的目錄資料。
    pub const fn spec(self) -> TopicSpec {
        use TopicCategory::{Chip, Dividend, Fundamental, Technical, Value};
        match self {
            Self::HoldingsExDividendPreview => holdings(
                "holdings.ex_dividend_preview",
                "持股除權息預告",
                "未來數日將除權息的持股",
                Dividend,
                true,
            ),
            Self::HoldingsExDividend => holdings(
                "holdings.ex_dividend",
                "持股除權息預估",
                "除權息當日，依持股估算可領金額",
                Dividend,
                true,
            ),
            Self::HoldingsPayable => holdings(
                "holdings.payable",
                "持股股利入帳",
                "股利發放日，列出入帳金額",
                Dividend,
                true,
            ),
            Self::HoldingsMonthlyPayout => holdings(
                "holdings.monthly_payout",
                "持股股利入帳彙總",
                "每月一次，上月入帳合計與預估誤差",
                Dividend,
                true,
            ),
            Self::HoldingsPayableFill => holdings(
                "holdings.payable_fill",
                "持股填息",
                "持股完成填息或逾期未填息",
                Dividend,
                false,
            ),
            Self::HoldingsMarketValue => holdings(
                "holdings.market_value",
                "市值變化",
                "交易日收盤後的市值增減",
                Value,
                true,
            ),
            Self::HoldingsRevenue => holdings(
                "holdings.revenue",
                "持股月營收",
                "持股公布月營收時",
                Value,
                true,
            ),
            Self::HoldingsFinancialReport => holdings(
                "holdings.financial_report",
                "持股財報",
                "持股公布季報時",
                Value,
                true,
            ),
            Self::HoldingsWeekly => holdings(
                "holdings.weekly",
                "每週持股摘要",
                "每週一次，週摘要裡全部的持股段落",
                Value,
                false,
            ),
            Self::HoldingsStrategyOverlap => holdings(
                "holdings.strategy_overlap",
                "策略換股的持股對照",
                "具名策略換股日，策略持股與實際持股的對照",
                Value,
                false,
            ),
            Self::HoldingsBrokerFlow => holdings(
                "holdings.broker_flow",
                "持股主力進出",
                "交易日收盤後，持股的主力分點買賣超",
                Chip,
                false,
            ),
            Self::HoldingsChipFlow => holdings(
                "holdings.chip_flow",
                "持股法人與信用交易",
                "交易日收盤後，三大法人與融資融券變化",
                Chip,
                false,
            ),
            Self::HoldingsHolderDistribution => holdings(
                "holdings.holder_distribution",
                "持股大戶週報",
                "每週集保股權分散，大戶持股增減",
                Chip,
                false,
            ),
            Self::HoldingsInsiderPledge => holdings(
                "holdings.insider_pledge",
                "持股董監質押",
                "董監持股與質押比例變動",
                Chip,
                false,
            ),
            Self::HoldingsForeignHolding => holdings(
                "holdings.foreign_holding",
                "持股外資持股",
                "持股的外資持股比例達到通知門檻時",
                Chip,
                false,
            ),
            Self::HoldingsEarningsCall => holdings(
                "holdings.earnings_call",
                "持股法說會",
                "持股即將召開或已召開法說會",
                Fundamental,
                true,
            ),
            Self::HoldingsFinancialHealth => holdings(
                "holdings.financial_health",
                "持股財務體質變化",
                "財務體質評等有變動時",
                Fundamental,
                true,
            ),
            Self::HoldingsValuation => holdings(
                "holdings.valuation",
                "持股估值週報",
                "每週一次，估值偏高或偏低的持股",
                Fundamental,
                false,
            ),
            Self::HoldingsEtfPremium => holdings(
                "holdings.etf_premium",
                "持有 ETF 折溢價與申贖",
                "持有的 ETF 折溢價過大或大額申贖",
                Fundamental,
                false,
            ),
            Self::HoldingsTechnical => holdings(
                "holdings.technical",
                "持股技術訊號",
                "交易日收盤後，持股出現的技術訊號",
                Technical,
                true,
            ),
            Self::HoldingsBreakout => holdings(
                "holdings.breakout",
                "區間突破（標示持股）",
                "全市場區間突破名單，其中的持股會標示",
                Technical,
                false,
            ),
            Self::MarketRegime => market(
                "market.regime",
                "大盤趨勢變化",
                "大盤狀態、廣度或估值水位轉變時",
            ),
            Self::MarketWeekly => market(
                "market.weekly",
                "每週市場摘要",
                "每週一次，大盤、產業資金、突破、ETF 申贖與策略段落",
            ),
            Self::MarketStrategyRebalance => market(
                "market.strategy_rebalance",
                "策略換股",
                "具名策略換股日的進出名單",
            ),
            Self::MarketStrategyEdge => market(
                "market.strategy_edge",
                "策略優勢檢查",
                "策略近期表現相對基準的變化",
            ),
            Self::MarketPublicOffering => {
                market("market.public_offering", "公開申購", "有可申購的股票時")
            }
            Self::MarketExDividend => market(
                "market.ex_dividend",
                "今日除權息",
                "全市場今天與下一交易日除權息的股票與歷年填息",
            ),
            Self::PersonalPriceTrace => TopicSpec {
                code: "personal.price_trace",
                name: "我的價格追蹤",
                description: "清單中的股票觸及設定價位；內容依收件者產生",
                category: TopicCategory::PriceTrace,
                audience: Audience::Personal,
                required_permission: PRICE_TRACE_VIEW,
                root_only: false,
                default_enabled: true,
            },
            Self::SystemCritical => TopicSpec {
                root_only: true,
                ..system(
                    "system.critical",
                    "系統關鍵警報",
                    "排程失敗、資料庫連線失敗、來源封鎖等需要立即處理的狀況",
                )
            },
            Self::SystemStartup => system("system.startup", "服務啟動通知", "服務啟動或重啟完成時"),
            Self::SystemAppUpdate => system(
                "system.app_update",
                "程式更新通知",
                "以機器人指令更新程式時",
            ),
            Self::SystemDbHealth => {
                system("system.db_health", "資料庫健康", "資料庫狀態異常或恢復")
            }
            Self::SystemHostHealth => {
                system("system.host_health", "主機健康", "pi 主機資源或狀態異常")
            }
            Self::SystemClosingCheck => system(
                "system.closing_check",
                "收盤資料檢查",
                "交易日收盤後，報價與衍生資料的檢查結果",
            ),
            Self::SystemAfterHours => system(
                "system.after_hours",
                "盤後作業結果",
                "盤後各項資料作業的完成摘要",
            ),
            Self::SystemDataHealth => system(
                "system.data_health",
                "資料健康週報",
                "每週一次的資料完整性報告",
            ),
        }
    }

    /// 項目代碼，例如 `holdings.payable`。
    pub const fn code(self) -> &'static str {
        self.spec().code
    }

    /// 受眾。
    pub const fn audience(self) -> Audience {
        self.spec().audience
    }
}

#[cfg(test)]
mod tests;
