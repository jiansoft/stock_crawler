//! `/stocks/screen/backtest` 的 request、response 與 OpenAPI schema。
//!
//! 選股條件只用換股當天就已知道的資料：每天留存的相對強度、法人連續買超、技術訊號，以及依法定
//! 公布期限判斷當時已公布的月營收與季報（見 `min_revenue_yoy_percent`、`min_eps`、
//! `min_gross_margin_percent`）；報酬來自
//! `daily_forward_return`（基準日收盤到之後第 N 個交易日收盤的還原報酬，等同含息；期間內下市的股票
//! 算到最後一筆收盤）。
//!
//! 子模組依端點放 response：[`offsets`]（換股日偏移）、[`frontier`]（0050 比例前緣）、
//! [`attribution`]（報酬歸因）、[`walk_forward`]（滾動前推）、[`preset`]（具名策略清單）；
//! 回測本身的回應在 [`response`]。

mod attribution;
mod frontier;
mod offsets;
mod preset;
mod response;
mod track;
mod walk_forward;

use serde::Deserialize;
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
pub(crate) use self::response::{
    BacktestContributor, BacktestContributors, BacktestCurrent, BacktestExit, BacktestHolding,
    BacktestPeriod, BacktestSummary, BacktestYear, ScreenBacktestResponse,
};
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
    /// 換股當天官方本益比由低到高（虧損或沒有資料的排最後）。
    PeRatio,
    /// 換股當天官方股價淨值比由低到高（沒有資料的排最後）。
    PbRatio,
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
    /// 大盤估值濾網（倍，5–100）：換股日同市場（`market`）官方本益比中位數（`/market/valuation` 的每日值）
    /// 高於 N 倍時這期空手，上一期持股全部賣出；可與其他濾網併用（都通過才持股）。資料從 2012 年起，
    /// 換股日沒有估值資料時視為通過；省略為不濾。
    #[param(minimum = 5, maximum = 100)]
    pub(crate) market_filter_max_pe: Option<f64>,
    /// 相對估值濾網（百分位，1–99）：換股日同市場本益比中位數在換股日之前近 5 年（不含當天）的每日中位數中，
    /// 低於當天值的比例達 N% 時這期空手，上一期持股全部賣出。只用換股日以前的資料，市場本益比長期上移時
    /// 門檻也會跟著移動；之前不足 250 個交易日或換股日沒有估值資料時視為通過。可與其他濾網併用；省略為不濾。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) market_filter_pe_percentile: Option<u8>,
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
    /// 最近一季營業利益率）、`pe_ratio`／`pb_ratio`（換股當天官方本益比／股價淨值比由低到高，虧損或沒有
    /// 資料的排最後）；同值或查不到時再依相對強度分數（查不到的排最後）。`keep_rank` 的名次也依此排序。
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
    /// 換股當天已公布的最近一個月營收至少這麼多（新台幣千元，0–1000000000000，與 `/stocks/screen` 的
    /// `min_monthly_revenue` 同單位）；公布日規則同 `min_revenue_yoy_percent`。用來排除營收規模太小、
    /// 年增率或利潤率因低基期失真的股票（例如依 `rank_by=revenue_yoy`、`operating_margin` 排序時）。
    #[param(minimum = 0.0, maximum = 1000000000000.0)]
    pub(crate) min_monthly_revenue: Option<f64>,
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
    /// `true` 時只選換股當天已公布的最新月營收創歷史新高（高於之前所有月份，之前至少 12 個月）的股票；
    /// 公布日規則同 `min_revenue_yoy_percent`；`false` 或省略不篩選。
    pub(crate) revenue_record: Option<bool>,
    /// 換股當天已公布的最近一季季末負債比（總負債 ÷ 總資產）不超過這個值（%，0–1000）。用 Yahoo 單季
    /// 資產負債表（2015 年起），公布日規則同 `min_eps`；沒有資料的股票不會通過。
    #[param(minimum = 0.0, maximum = 1000.0)]
    pub(crate) max_debt_ratio_percent: Option<f64>,
    /// `true` 時只選換股當天已公布的最近四季自由現金流合計為正的股票。用 Yahoo 單季現金流量表（2015 年起），
    /// 最近一季的公布日規則同 `min_eps`，連同之前 3 季須四季都有資料，否則不會通過；`false` 或省略不篩選。
    pub(crate) positive_free_cash_flow: Option<bool>,
    /// 換股當天的官方本益比（證交所／櫃買中心公布的日資料）不超過這個值（倍，0–10000）。本益比為 0
    /// （近四季虧損或當天沒有資料）的股票不會通過；換股當天收盤後才公布，與以收盤價買進同一時點。
    #[param(minimum = 0.0, maximum = 10000.0)]
    pub(crate) max_pe: Option<f64>,
    /// 換股當天的官方股價淨值比不超過這個值（倍，0–1000）；為 0（沒有資料）的股票不會通過，其餘同
    /// `max_pe`。
    #[param(minimum = 0.0, maximum = 1000.0)]
    pub(crate) max_pb: Option<f64>,
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
