//! `/stocks/screen/backtest` 的 request、response 與 OpenAPI schema。
//!
//! 選股條件只用換股當天就已知道的資料：每天留存的相對強度、法人連續買超、技術訊號，以及依法定
//! 公布期限判斷當時已公布的月營收與季報（見 `min_revenue_yoy_percent`、`min_eps`、
//! `min_gross_margin_percent`）；報酬來自
//! `daily_forward_return`（基準日收盤到之後第 N 個交易日收盤的還原報酬，等同含息；期間內下市的股票
//! 算到最後一筆收盤）。
//!
//! 子模組依端點放 response：[`offsets`]（換股日偏移）、[`frontier`]（0050 比例前緣）、
//! [`attribution`]（報酬歸因）、[`walk_forward`]（滾動前推）、[`preset`]（具名策略清單）。

mod attribution;
mod frontier;
mod offsets;
mod preset;
mod track;
mod walk_forward;

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{MarketParamValue, SignalIndicatorValue, SignalKindValue};

pub(crate) use self::attribution::{
    BacktestAttributionTotals, BacktestAttributionYear, BacktestIndustryEffect,
    ScreenBacktestAttributionResponse,
};
pub(crate) use self::frontier::{BacktestFrontierPoint, ScreenBacktestFrontierResponse};
pub(crate) use self::offsets::{
    BacktestOffsetDistribution, BacktestOffsetResult, BacktestOffsetYear, BacktestStat,
    ScreenBacktestOffsetsResponse,
};
pub(crate) use self::preset::{StrategyPresetItem, StrategyPresetsResponse};
pub(crate) use self::track::{StrategyTrackHolding, StrategyTrackPeriod, StrategyTrackResponse};
pub(crate) use self::walk_forward::{BacktestWalkForwardYear, ScreenBacktestWalkForwardResponse};

/// OpenAPI 文件使用的候選股排序方式。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
enum BacktestRankByValue {
    /// 相對強度分數由高到低。
    RsScore,
    /// 換股日含當天往前 20 個交易日的平均成交金額由高到低。
    TradeValue,
    /// 外資連續買超天數由長到短。
    ForeignStreak,
    /// 投信連續買超天數由長到短。
    TrustStreak,
    /// 換股日已公布的最新月營收年增率由高到低。
    RevenueYoy,
    /// 換股日已公布的最近一季（Yahoo 單季財報）營業利益率由高到低。
    OperatingMargin,
}

/// OpenAPI 文件使用的持股加權方式。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
enum BacktestWeightingValue {
    /// 等權。
    Equal,
    /// 依換股日含當天往前 20 個交易日的平均成交金額加權（接近市值加權）。
    TradeValue,
    /// 依平均成交金額的平方根加權（介於等權與成交金額加權之間）。
    SqrtTradeValue,
}

/// 條件選股回測的 query string。
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ScreenBacktestParams {
    /// 具名策略（見 `/strategy-presets`）：以存的條件為準，請求另帶的參數覆蓋同名參數，例如
    /// `preset=最佳組合&rebalance_offset=3`；查無策略回 404。
    pub(crate) preset: Option<String>,
    /// 回測起日（`YYYY-MM-DD`）；省略為迄日往前 1,095 天（約三年）。
    pub(crate) from: Option<String>,
    /// 回測迄日（`YYYY-MM-DD`）；省略為最新有相對強度的交易日。起訖最長 3,653 天（約十年）。
    pub(crate) to: Option<String>,
    /// 持有天數（交易日）：5、20（預設）、60、120 或 240；每隔這麼多個交易日換股一次，
    /// 持有到下一次換股，報酬為這段期間的還原報酬。
    #[param(default = 20)]
    pub(crate) hold_days: Option<u16>,
    /// 每期最多持有幾檔（1–100，預設 20）：符合條件的股票依 `rank_by`（預設相對強度分數）由高到低取前
    /// N 檔，等權持有。
    #[param(minimum = 1, maximum = 100, default = 20)]
    pub(crate) limit: Option<u8>,
    /// 換股緩衝（`limit`–200，預設等於 `limit` 即不緩衝）：上一期持有的股票只要這期仍符合條件、
    /// 且排在前 `keep_rank` 名就續抱，空出的名額才依相對強度補滿，用來降低換手與交易成本。
    #[param(minimum = 1, maximum = 200)]
    pub(crate) keep_rank: Option<u8>,
    /// 大盤濾網：換股日加權指數收盤低於自己的 N 日均線（20、60、120 或 240 個交易日）時這期空手，
    /// 上一期持股全部賣出；省略為不濾。
    pub(crate) market_filter_ma: Option<u16>,
    /// 市場廣度濾網（1–99）：換股日同市場站上 60 日均線的股票比例低於 N% 時這期空手，上一期持股
    /// 全部賣出；可與 `market_filter_ma` 併用（兩者都通過才持股）。統計從 2019 年起；省略為不濾。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) market_filter_breadth: Option<u8>,
    /// 續抱條件（1–99）：上一期持有的股票即使不再符合選股條件（例如法人連買中斷、訊號過期），只要
    /// 換股當天相對強度分數不低於此值就續抱；省略為不續抱（只看 `keep_rank`）。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) hold_min_rs_score: Option<u8>,
    /// 同產業最多持有幾檔（1–100）：每期持股（含續抱）同一產業超過 N 檔時，排名較後的跳過、
    /// 改由其他產業依相對強度補滿，用來降低產業集中；省略為不限。
    #[param(minimum = 1, maximum = 100)]
    pub(crate) max_per_industry: Option<u8>,
    /// 換股日偏移（0 到 `hold_days` − 1，預設 0）：起日之後先跳過幾個交易日才第一次換股。
    /// 同一組條件換不同偏移重跑，可看結果是否只是剛好碰上某幾個換股日。
    #[param(minimum = 0, maximum = 239)]
    pub(crate) rebalance_offset: Option<u16>,
    /// 流動性門檻（百萬元，1–100000）：換股日含當天往前 20 個交易日的平均成交金額低於此值的股票
    /// 不新買進（上一期持股的續抱不受限）；例如 50 代表日均成交 5,000 萬元。省略為不限。
    #[param(minimum = 1, maximum = 100000)]
    pub(crate) min_avg_trade_value: Option<u32>,
    /// 符合條件的股票依什麼排序取前 `limit` 檔：`rs_score`（預設，相對強度分數）、`trade_value`
    /// （近 20 個交易日平均成交金額，偏向大型股）、`foreign_streak`／`trust_streak`（外資／投信連續
    /// 買超天數）、`revenue_yoy`（換股日已公布的最新月營收年增率）、`operating_margin`（換股日已公布的
    /// 最近一季營業利益率）；同值或查不到時再依相對強度分數（查不到的排最後）。`keep_rank` 的名次也依此排序。
    #[param(value_type = BacktestRankByValue, inline)]
    pub(crate) rank_by: Option<String>,
    /// 0050 核心部位（%，0–100，預設 0）：組合固定以這個比例持有 0050、其餘才依條件選股，
    /// 用來看策略搭配大盤的效果；彙總為混合後的組合報酬，每期換股時調回固定比例的買賣計入成本
    /// （0050 證交稅 0.1%，第一次買進 0050 不計），空手時 0050 照常持有。100 等於只持有 0050。
    #[param(minimum = 0, maximum = 100, default = 0)]
    pub(crate) etf_weight: Option<u8>,
    /// 策略部位的持股加權方式：`equal`（預設，等權）、`trade_value`（依換股日含當天往前 20 個交易日的
    /// 平均成交金額，接近市值加權、偏重大型股）或 `sqrt_trade_value`（依平均成交金額的平方根，介於
    /// 兩者之間）。換手成本依每期目標權重與上一期持股漲跌後權重的差額計算，續抱股票的權重調整也計入。
    #[param(value_type = BacktestWeightingValue, inline)]
    pub(crate) weighting: Option<String>,
    /// 單檔權重上限（%，1–100，選填）：策略部位中每檔最多占這個比例，超出的部分依權重分給其他持股；
    /// 檔數 × 上限不到 100%（例如上限 10% 只選到 5 檔）時剩下的放現金。依成交金額加權時用來避免集中在
    /// 少數大型股。
    #[param(minimum = 1, maximum = 100)]
    pub(crate) max_weight: Option<u8>,
    /// 停損（%，1–50，選填）：持有期間收盤相對買進價跌幅達這個比例，當天收盤賣出、該檔到下次換股前
    /// 放現金（賣出付手續費與證交稅，下次仍入選視為重新買進）。以每天的還原收盤檢查，只用於已滿持有
    /// 天數的期數，目前持股不檢查。
    #[param(minimum = 1, maximum = 50)]
    pub(crate) stop_loss: Option<u8>,
    /// 移動停利（%，1–50，選填）：持有期間收盤相對這期最高收盤（含買進價）回落達這個比例就賣出，
    /// 規則同 `stop_loss`；兩者都設時先觸發的為準。
    #[param(minimum = 1, maximum = 50)]
    pub(crate) trailing_stop: Option<u8>,
    /// 滾動前推的訓練年數（1–5，預設 3，只用於 `/walk-forward`）：每年用之前這麼多年的期數挑參數，
    /// 套用到這一年。
    #[param(minimum = 1, maximum = 5, default = 3)]
    pub(crate) train_years: Option<u8>,
    /// 樣本內外分割日（`YYYY-MM-DD`，選填，只用於 `/offsets` 與 `/frontier`）：換股日早於這天的期數為
    /// 樣本內、其餘為樣本外，各自再算一次分布，用來看在前段調出來的條件到後段是否仍然成立。必須晚於
    /// 起日、不晚於迄日。
    pub(crate) split: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`；大盤基準也依此市場計算。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 只選這個產業（產業名稱，例如「半導體業」）的股票；大盤基準不受影響。
    pub(crate) industry: Option<String>,
    /// 排除的產業（產業名稱，多個以逗號分隔，最多 10 個，例如「金融保險業,航運業」）：這些產業的股票
    /// 不買進；大盤基準不受影響。不可與 `industry` 相同。
    pub(crate) exclude_industry: Option<String>,
    /// 換股當天相對強度分數至少這麼高（1–99）。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) min_rs_score: Option<u8>,
    /// 換股當天相對強度分數至多這麼高（1–99）。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) max_rs_score: Option<u8>,
    /// 換股當天外資（含外資自營商）已連續買超至少這麼多天（1–60），算法同
    /// `/market/institutional-flow`；籌碼資料從 2019 年起。
    #[param(minimum = 1, maximum = 60)]
    pub(crate) min_foreign_streak: Option<u8>,
    /// 換股當天投信已連續買超至少這麼多天（1–60）；規則同 `min_foreign_streak`。
    #[param(minimum = 1, maximum = 60)]
    pub(crate) min_trust_streak: Option<u8>,
    /// 換股當天已公布的最近一個月營收年增率至少這麼高（%，-100–10000）。每月營收最晚次月 10 日公布，
    /// 次月 11 日起才使用（例如 9 月營收從 10 月 11 日起），最近一筆已公布的月份距換股日超過 3 個月
    /// 視為沒有資料、不會通過。只用於新買進，續抱不看基本面條件。
    #[param(minimum = -100.0, maximum = 10000.0)]
    pub(crate) min_revenue_yoy_percent: Option<f64>,
    /// 換股當天已公布的月營收從最近一個月往回連續年增（> 0、月份相連）至少這麼多個月（1–12）；
    /// 公布日規則同 `min_revenue_yoy_percent`。
    #[param(minimum = 1, maximum = 12)]
    pub(crate) min_revenue_yoy_streak: Option<u8>,
    /// 換股當天已公布的最近 3 個月（須相連）合計營收對去年同期的年增率至少這麼高（%，-100–10000）；
    /// 公布日規則同 `min_revenue_yoy_percent`。
    #[param(minimum = -100.0, maximum = 10000.0)]
    pub(crate) min_revenue_yoy_3m_percent: Option<f64>,
    /// 換股當天已公布的最近一季每股盈餘至少這麼高（元，-10000–10000）。季報以法定期限的隔天起才使用：
    /// Q1 5/16、Q2 8/15、Q3 11/15、Q4 隔年 4/1；最近一季距換股日超過 3 季視為沒有資料。財報從 2018
    /// 年起，且為目前留存的數字（事後更正會沿用更正後的值）。
    #[param(minimum = -10000.0, maximum = 10000.0)]
    pub(crate) min_eps: Option<f64>,
    /// 換股當天已公布的最近一季股東權益報酬率至少這麼高（%，-10000–10000，與 `/stocks/screen` 的
    /// `min_roe_percent` 同一欄位）；公布日規則同 `min_eps`。
    #[param(minimum = -10000.0, maximum = 10000.0)]
    pub(crate) min_roe_percent: Option<f64>,
    /// 換股當天已公布的最近一季單季毛利率（毛利 ÷ 營收）至少這麼高（%，-10000–100）。用 Yahoo 單季損益表
    /// （2015 年起），公布日規則同 `min_eps`；營收為 0 或沒有資料的股票不會通過。
    #[param(minimum = -10000.0, maximum = 100.0)]
    pub(crate) min_gross_margin_percent: Option<f64>,
    /// 換股當天已公布的最近一季單季營業利益率（營業利益 ÷ 營收）至少這麼高（%，-10000–100）；資料與
    /// 公布日規則同 `min_gross_margin_percent`。
    #[param(minimum = -10000.0, maximum = 100.0)]
    pub(crate) min_operating_margin_percent: Option<f64>,
    /// `true` 時只選「三率三升」：換股當天已公布的最近一季毛利率、營業利益率與稅後淨利率都高於去年
    /// 同一季；去年同季沒有資料的不會通過。資料與公布日規則同 `min_gross_margin_percent`；`false` 或
    /// 省略不篩選。
    pub(crate) margins_yoy_up: Option<bool>,
    /// 換股當天已公布的最近一季季末負債比（總負債 ÷ 總資產）不超過這個值（%，0–1000）。用 Yahoo 單季
    /// 資產負債表（2015 年起），公布日規則同 `min_eps`；沒有資料的股票不會通過。
    #[param(minimum = 0.0, maximum = 1000.0)]
    pub(crate) max_debt_ratio_percent: Option<f64>,
    /// 只選換股前 `hold_days` 個交易日內（含換股當天）出現過這個來源訊號的股票。
    #[param(value_type = Option<SignalIndicatorValue>)]
    pub(crate) indicator: Option<String>,
    /// 只選換股前 `hold_days` 個交易日內出現過這種訊號的股票；可與 `indicator` 併用。
    #[param(value_type = Option<SignalKindValue>)]
    pub(crate) kind: Option<String>,
    /// 手續費折扣（0–1，預設 1 為不打折；例如 0.28 為 2.8 折）：扣成本後的報酬以手續費 0.1425%
    /// 乘上折扣、賣出另加證交稅 0.3% 計算。
    #[param(minimum = 0.0, maximum = 1.0, default = 1.0)]
    pub(crate) fee_discount: Option<f64>,
}

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
    /// 最近一季負債比上限（%）；未篩選為 `null`。
    pub(crate) max_debt_ratio_percent: Option<f64>,
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
