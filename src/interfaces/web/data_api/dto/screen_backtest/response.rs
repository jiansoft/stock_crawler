//! `/stocks/screen/backtest` 的回應：各期、彙總、年度、個股貢獻與目前持股。

use serde::Serialize;
use utoipa::ToSchema;

/// 一次換股（一期）的結果。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestPeriod {
    /// 換股日（收盤買進）。
    pub(crate) date: String,
    /// 這期持有的檔數；0 代表沒有符合條件的股票或大盤濾網沒過，這期空手（報酬 0）。
    pub(crate) holdings: i64,
    /// 換股日加權指數低於濾網均線或市場廣度低於門檻而空手。
    pub(crate) market_filtered: bool,
    /// 這期在持有期間觸發停損或移動停利、提前賣出的檔數；沒設停損為 0。
    pub(crate) stopped: i64,
    /// 持股等權平均報酬（%），未扣交易成本；期間內下市的股票算到最後一筆收盤，之後完全沒有收盤的
    /// 股票不列入平均；空手為 `null`。
    pub(crate) avg_return: Option<f64>,
    /// 扣除這次換股的手續費與證交稅後的報酬（%）；空手但賣出上一期持股時為負的成本。
    pub(crate) return_after_costs: f64,
    /// 換手率（0–1）：這期新買進與加碼（含續抱股票調回目標權重）占組合的比例，第一期為 1、空手為 0。
    pub(crate) turnover: f64,
    /// 同一天同市場全部母體股票的平均報酬（%）。
    pub(crate) benchmark_return: Option<f64>,
    /// 同一天買進 0050 持有同樣天數的還原報酬（%）；缺資料為 `null`。
    pub(crate) etf_return: Option<f64>,
    /// 依相對強度分數由高到低的前 10 檔代號（換手率以全部持股計算）。
    pub(crate) symbols: Vec<String>,
}

/// 整段回測的彙總；沒有任何一期時數值為 `null`。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestSummary {
    /// 期數。
    pub(crate) periods: i64,
    /// 有持股的期數。
    pub(crate) invested_periods: i64,
    /// 因大盤濾網（均線或市場廣度）空手的期數。
    pub(crate) market_filtered_periods: i64,
    /// 各期觸發停損或移動停利、提前賣出的總檔次；沒設停損為 0。
    pub(crate) stop_exits: i64,
    /// 有持股的期平均持有檔數。
    pub(crate) avg_holdings: Option<f64>,
    /// 有持股的期平均換手率（0–1）。
    pub(crate) avg_turnover: Option<f64>,
    /// 累積報酬（%）：各期報酬連乘，空手的期數報酬為 0；未扣交易成本。
    pub(crate) total_return: Option<f64>,
    /// 扣除交易成本後的累積報酬（%）。
    pub(crate) total_return_after_costs: Option<f64>,
    /// 大盤基準的累積報酬（%）。
    pub(crate) benchmark_total_return: Option<f64>,
    /// 年化報酬（%），以每年約 245 個交易日換算；未扣交易成本。
    pub(crate) annualized_return: Option<f64>,
    /// 扣除交易成本後的年化報酬（%）。
    pub(crate) annualized_return_after_costs: Option<f64>,
    /// 大盤基準的年化報酬（%）。
    pub(crate) benchmark_annualized_return: Option<f64>,
    /// 最大回撤（%，負值）：以每期期末的累積淨值計算，看不到期中的波動。
    pub(crate) max_drawdown: Option<f64>,
    /// 扣除交易成本後的最大回撤（%）。
    pub(crate) max_drawdown_after_costs: Option<f64>,
    /// 大盤基準的最大回撤（%）。
    pub(crate) benchmark_max_drawdown: Option<f64>,
    /// 報酬大於 0 的期數比例（0–1），未扣交易成本。
    pub(crate) win_rate: Option<f64>,
    /// 報酬高於大盤基準的期數比例（0–1）。
    pub(crate) beat_market_rate: Option<f64>,
    /// 每期平均報酬（%，空手算 0）。
    pub(crate) avg_period_return: Option<f64>,
    /// 大盤基準的每期平均報酬（%）。
    pub(crate) avg_benchmark_return: Option<f64>,
    /// 每期都買 0050 的累積報酬（%）：實際買得到的大盤基準，未扣交易成本；全期都缺資料為 `null`。
    pub(crate) etf_total_return: Option<f64>,
    /// 0050 的年化報酬（%）。
    pub(crate) etf_annualized_return: Option<f64>,
    /// 0050 的最大回撤（%）。
    pub(crate) etf_max_drawdown: Option<f64>,
    /// 報酬高於 0050 的期數比例（0–1）。
    pub(crate) beat_etf_rate: Option<f64>,
    /// 年化波動率（%）：每期報酬（未扣成本）的標準差 × √每年期數；不足 2 期為 `null`。
    pub(crate) volatility: Option<f64>,
    /// 夏普值：扣交易成本後每期平均報酬 ÷ 每期標準差 × √每年期數，無風險利率以 0 計；
    /// 不足 2 期或報酬沒有波動為 `null`。
    pub(crate) sharpe_ratio: Option<f64>,
    /// 大盤基準的年化波動率（%）。
    pub(crate) benchmark_volatility: Option<f64>,
    /// 大盤基準的夏普值（未扣成本）。
    pub(crate) benchmark_sharpe_ratio: Option<f64>,
    /// 0050 的年化波動率（%）；全期都缺資料為 `null`。
    pub(crate) etf_volatility: Option<f64>,
    /// 0050 的夏普值（未扣成本）。
    pub(crate) etf_sharpe_ratio: Option<f64>,
}

/// 依換股日所屬年度彙總的一年（跨年的那一期算在換股日的年度）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestYear {
    /// 年度。
    pub(crate) year: i32,
    /// 這一年換股的期數。
    pub(crate) periods: i64,
    /// 這一年各期報酬連乘的累積報酬（%），空手算 0；未扣交易成本。
    pub(crate) total_return: f64,
    /// 扣除交易成本後的累積報酬（%）。
    pub(crate) total_return_after_costs: f64,
    /// 大盤基準（同市場全部母體平均）的累積報酬（%）。
    pub(crate) benchmark_total_return: f64,
    /// 0050 的累積報酬（%）；這一年都缺資料為 `null`。
    pub(crate) etf_total_return: Option<f64>,
    /// 報酬大於 0 的期數比例（0–1）。
    pub(crate) win_rate: f64,
    /// 報酬高於大盤基準的期數比例（0–1）。
    pub(crate) beat_market_rate: f64,
}

/// 整段回測中一檔股票的貢獻。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestContributor {
    /// 股票代號。
    pub(crate) symbol: String,
    /// 股票名稱；查無為 `null`。
    pub(crate) name: Option<String>,
    /// 持有過的期數。
    pub(crate) periods_held: i64,
    /// 累計貢獻（百分點）：各期「這檔報酬 ÷ 當期有報酬的持股數」相加，不複利、未扣交易成本。
    pub(crate) contribution: f64,
    /// 持有各期的平均報酬（%），沒有報酬的期不計；全部沒有為 `null`。
    pub(crate) avg_return: Option<f64>,
}

/// 個股貢獻：報酬集中在哪幾檔、哪幾檔拖累最多。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestContributors {
    /// 整段回測持有過的不同股票數。
    pub(crate) held_symbols: i64,
    /// 累計貢獻為正的前 10 檔，由大到小。
    pub(crate) best: Vec<BacktestContributor>,
    /// 累計貢獻為負的前 10 檔，由最拖累起。
    pub(crate) worst: Vec<BacktestContributor>,
}

/// 目前持股的一檔。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestHolding {
    /// 股票代號。
    pub(crate) symbol: String,
    /// 股票名稱；查無為 `null`。
    pub(crate) name: Option<String>,
    /// 在策略部位中的權重（%）；等權時各檔相同，有現金時合計不到 100。
    pub(crate) weight: f64,
    /// 這次換股新買進（上一期沒有持有）。
    pub(crate) added: bool,
    /// 換股日收盤到迄日的還原報酬（%）；查不到日線為 `null`。
    pub(crate) return_to_date: Option<f64>,
}

/// 這次換股賣出的一檔（上一期持有、這一期沒有）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestExit {
    /// 股票代號。
    pub(crate) symbol: String,
    /// 股票名稱；查無為 `null`。
    pub(crate) name: Option<String>,
    /// 上一期換股時在策略部位中的權重（%）。
    pub(crate) weight: f64,
}

/// 最後一次換股、還沒滿持有天數的持股，也就是依策略現在應該持有的股票。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestCurrent {
    /// 換股日。
    pub(crate) date: String,
    /// 換股日之後到回測迄日經過的交易日數。
    pub(crate) days_elapsed: i64,
    /// 距離下次換股還有幾個交易日。
    pub(crate) days_to_next_rebalance: i64,
    /// 大盤濾網沒過、這期空手。
    pub(crate) market_filtered: bool,
    /// 相對上一期新買進的部位比例（0–1）。
    pub(crate) turnover: f64,
    /// 策略部位放現金的比例（%）：設 `max_weight` 且檔數 × 上限不到 100% 時才大於 0。
    pub(crate) cash: f64,
    /// 全部持股，依相對強度由高到低；空手為空陣列。
    pub(crate) holdings: Vec<BacktestHolding>,
    /// 這次換股賣出的股票（上一期持有、這一期沒有）；沒有上一期為空陣列。
    pub(crate) exited: Vec<BacktestExit>,
    /// 策略部位換股日收盤到迄日的還原報酬（%）：持股依權重的加權平均乘上持股比例，未扣交易成本、
    /// 不含 0050 核心部位；持股都查不到日線或空手時為 `null`。
    pub(crate) return_to_date: Option<f64>,
    /// 0050 同期（換股日收盤到迄日）的還原報酬（%）；查不到為 `null`。
    pub(crate) etf_return_to_date: Option<f64>,
}

/// 條件選股回測結果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenBacktestResponse {
    /// 引用的具名策略；沒有用 `preset` 為 `null`。
    pub(crate) preset: Option<String>,
    /// 回測起日。
    pub(crate) from: String,
    /// 回測迄日。
    pub(crate) to: String,
    /// 市場：`all`、`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 持有天數（交易日）。
    pub(crate) hold_days: u16,
    /// 每期最多持有檔數。
    pub(crate) limit: u8,
    /// 續抱的排名緩衝；等於 `limit` 為不緩衝。
    pub(crate) keep_rank: u8,
    /// 大盤濾網的均線天數；未設定為 `null`。
    pub(crate) market_filter_ma: Option<u16>,
    /// 市場廣度濾網門檻（%）；未設定為 `null`。
    pub(crate) market_filter_breadth: Option<u8>,
    /// 大盤估值濾網門檻（市場本益比中位數上限，倍）；未設定為 `null`。
    pub(crate) market_filter_max_pe: Option<f64>,
    /// 相對估值濾網門檻（近 5 年百分位）；未設定為 `null`。
    pub(crate) market_filter_pe_percentile: Option<u8>,
    /// 續抱的相對強度門檻；未設定為 `null`。
    pub(crate) hold_min_rs_score: Option<u8>,
    /// 同產業最多持有檔數；未設定為 `null`。
    pub(crate) max_per_industry: Option<u8>,
    /// 換股日偏移（交易日）。
    pub(crate) rebalance_offset: u16,
    /// 流動性門檻（百萬元，20 日平均成交金額）；未設定為 `null`。
    pub(crate) min_avg_trade_value: Option<u32>,
    /// 候選股排序方式。
    pub(crate) rank_by: String,
    /// 0050 核心部位（%）；0 為純策略。
    pub(crate) etf_weight: u8,
    /// 策略部位的持股加權方式。
    pub(crate) weighting: String,
    /// 單檔權重上限（%）；未設定為 `null`。
    pub(crate) max_weight: Option<u8>,
    /// 停損（%）；未設定為 `null`。
    pub(crate) stop_loss: Option<u8>,
    /// 移動停利（%）；未設定為 `null`。
    pub(crate) trailing_stop: Option<u8>,
    /// 篩選的產業；未篩選為 `null`。
    pub(crate) industry: Option<String>,
    /// 排除的產業（逗號分隔）；未排除為 `null`。
    pub(crate) exclude_industry: Option<String>,
    /// 相對強度分數下限；未篩選為 `null`。
    pub(crate) min_rs_score: Option<u8>,
    /// 相對強度分數上限；未篩選為 `null`。
    pub(crate) max_rs_score: Option<u8>,
    /// 外資連續買超天數下限；未篩選為 `null`。
    pub(crate) min_foreign_streak: Option<u8>,
    /// 投信連續買超天數下限；未篩選為 `null`。
    pub(crate) min_trust_streak: Option<u8>,
    /// 月營收年增率下限（%）；未篩選為 `null`。
    pub(crate) min_revenue_yoy_percent: Option<f64>,
    /// 月營收連續年增月數下限；未篩選為 `null`。
    pub(crate) min_revenue_yoy_streak: Option<u8>,
    /// 近 3 個月營收年增率下限（%）；未篩選為 `null`。
    pub(crate) min_revenue_yoy_3m_percent: Option<f64>,
    /// 最新月營收下限（千元）；未篩選為 `null`。
    pub(crate) min_monthly_revenue: Option<f64>,
    /// 最近一季每股盈餘下限（元）；未篩選為 `null`。
    pub(crate) min_eps: Option<f64>,
    /// 最近一季股東權益報酬率下限（%）；未篩選為 `null`。
    pub(crate) min_roe_percent: Option<f64>,
    /// 最近一季毛利率下限（%）；未篩選為 `null`。
    pub(crate) min_gross_margin_percent: Option<f64>,
    /// 最近一季營業利益率下限（%）；未篩選為 `null`。
    pub(crate) min_operating_margin_percent: Option<f64>,
    /// 只選三率三升；未篩選為 `false`。
    pub(crate) margins_yoy_up: bool,
    /// 只選最新月營收創歷史新高；未篩選為 `false`。
    pub(crate) revenue_record: bool,
    /// 最近一季負債比上限（%）；未篩選為 `null`。
    pub(crate) max_debt_ratio_percent: Option<f64>,
    /// 只選近四季自由現金流為正；未篩選為 `false`。
    pub(crate) positive_free_cash_flow: bool,
    /// 換股當天本益比上限（倍）；未篩選為 `null`。
    pub(crate) max_pe: Option<f64>,
    /// 換股當天股價淨值比上限（倍）；未篩選為 `null`。
    pub(crate) max_pb: Option<f64>,
    /// 訊號來源；未篩選為 `null`。
    pub(crate) indicator: Option<String>,
    /// 訊號種類；未篩選為 `null`。
    pub(crate) kind: Option<String>,
    /// 計算交易成本用的手續費折扣（0–1）。
    pub(crate) fee_discount: f64,
    /// 報酬是否還原權值；固定為 `true`。
    pub(crate) adjusted: bool,
    /// 可投資的大盤基準代號（固定為 `0050`）。
    pub(crate) etf_benchmark: String,
    /// 整段回測的彙總。
    pub(crate) summary: BacktestSummary,
    /// 依換股日年度的彙總，由舊到新；用來看策略在多頭、空頭年份是否穩定。
    pub(crate) years: Vec<BacktestYear>,
    /// 個股貢獻最多與拖累最多的股票。
    pub(crate) contributors: BacktestContributors,
    /// 最後一次換股、還沒滿持有天數的持股（策略目前的持股，不計入彙總與各期）；最後一期已滿或迄日
    /// 剛好沒有未滿的換股日為 `null`。
    pub(crate) current: Option<BacktestCurrent>,
    /// 各期結果，由舊到新；只列已滿持有天數的期數。
    pub(crate) periods: Vec<BacktestPeriod>,
}
