//! 回測引擎：驗證條件、查每期候選股、決定持股與扣交易成本；回測與偏移、前緣、滾動前推、歸因各端點共用。

use axum::{http::StatusCode, response::Response};
use chrono::NaiveDate;

use super::current::split_current;
use super::portfolio::FEE_RATE;
use super::portfolio::{PeriodCost, Weighting, blend_etf, select_holdings, trading_costs, weigh};
use super::sql::{
    BACKTEST_NEXT_OPEN_SQL, BACKTEST_SQL, BACKTEST_WITH_CHIP_NEXT_OPEN_SQL, BACKTEST_WITH_CHIP_SQL,
    LATEST_RS_DATE_SQL,
};
use super::stops::{
    BarCache, StopRule, apply_stops, load_holding_windows, load_open_ratios, populate_daily_returns,
};
use super::validate::{parse_excluded_industries, resolve_backtest_period, validate_fundamentals};
use super::{
    CandidateRow, HOLD_DAYS, MARKET_FILTER_DAYS, MAX_KEEP_RANK, MAX_TRADE_VALUE_MILLION, PeriodRow,
    RANK_BY,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::ScreenBacktestParams;
use crate::interfaces::web::data_api::handlers::signal_history::{
    validate_rs_range, validate_streaks,
};
use crate::interfaces::web::data_api::handlers::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    signal_filter_error,
};

/// 滑價上限（基點）。
pub(super) const MAX_SLIPPAGE_BPS: u16 = 200;

/// 成交價模型（`execution_price`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ExecutionPrice {
    /// 換股日收盤買賣（理想模型，預設）。
    #[default]
    Close,
    /// 次一交易日開盤買進、持有期滿的次一交易日開盤賣出。
    NextOpen,
}

impl ExecutionPrice {
    /// 解析查詢參數；未提供為 [`Self::Close`]。
    pub(super) fn parse(value: Option<&str>) -> Option<Self> {
        match value.map(str::trim) {
            None | Some("") | Some("close") => Some(Self::Close),
            Some("next_open") => Some(Self::NextOpen),
            Some(_) => None,
        }
    }

    /// 回應中揭露的成交價模型名稱。
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::NextOpen => "next_open",
        }
    }
}

/// 把滑價換算成等效的手續費折數：成本以 `FEE_RATE × 折數` 單邊計，滑價同樣按成交金額單邊計，
/// 所以 `折數 + 滑價(%) / FEE_RATE` 讓所有扣成本的地方一併計入滑價。
pub(super) fn cost_fee_discount(fee_discount: f64, slippage_bps: u16) -> f64 {
    fee_discount + f64::from(slippage_bps) / 100.0 / FEE_RATE
}

/// 驗證過的回測條件（換股日偏移除外，由呼叫端決定）；`/stocks/screen/backtest` 與
/// `/stocks/screen/backtest/offsets` 共用。
pub(super) struct Backtest<'a> {
    pub(super) params: &'a ScreenBacktestParams,
    pub(super) from: NaiveDate,
    /// 迄日（含）之前最近一個有相對強度的交易日。
    pub(super) to: NaiveDate,
    pub(super) market: &'a str,
    pub(super) market_id: i32,
    pub(super) hold_days: u16,
    pub(super) limit: u8,
    pub(super) keep_rank: u8,
    /// 使用者指定的手續費折扣（回應原樣揭露）；實際扣成本用 [`Self::cost_fee_discount`]。
    pub(super) fee_discount: f64,
    /// 成交價模型。
    pub(super) execution: ExecutionPrice,
    /// 單邊滑價（基點）。
    pub(super) slippage_bps: u16,
    /// 候選股排序方式（[`RANK_BY`] 其中之一）。
    pub(super) rank_by: &'a str,
    /// 0050 核心部位的比例（%，0 為純策略）。
    pub(super) etf_weight: u8,
    /// 策略部位的持股加權方式。
    pub(super) weighting: Weighting,
    /// 單檔權重上限（%）；未設定為不限。
    pub(super) max_weight: Option<u8>,
    /// 樣本內外分割日（只用於偏移穩健度與前緣）。
    pub(super) split: Option<NaiveDate>,
    /// 排除的產業名稱；空的為不排除。
    pub(super) excluded_industries: Vec<String>,
    /// 期間停損規則。
    pub(super) stops: StopRule,
    /// 期間停損用的日線，各換股日偏移共用。
    pub(super) bars: BarCache,
    /// 是否載入日線算日頻淨值路徑（日頻最大回撤）；多組回測的端點只用期末口徑，見
    /// [`Self::period_end_only`]。
    pub(super) daily_path: bool,
}

/// 一次回測混合 0050 之前的各期結果（含還沒滿持有天數的最後一期）；同一組候選股可依不同的
/// `etf_weight` 重複混合（見 [`Raw::finish`]）。
#[derive(Clone)]
pub(super) struct Raw {
    pub(super) rows: Vec<PeriodRow>,
    pub(super) costs: Vec<PeriodCost>,
    /// 最後一期還沒滿持有天數時，換股日之後到迄日經過的交易日數。
    pub(super) elapsed_days: Option<i64>,
    /// 手續費折扣，0050 再平衡成本也用這個折扣。
    pub(super) fee_discount: f64,
}

impl Raw {
    /// 以 `etf_weight`（%）混合 0050 核心部位，並把還沒滿持有天數的最後一期移到
    /// [`Simulation::current`]。
    pub(super) fn finish(mut self, etf_weight: u8) -> Simulation {
        blend_etf(
            &mut self.rows,
            &mut self.costs,
            f64::from(etf_weight) / 100.0,
            self.fee_discount,
        );
        let current = split_current(&mut self.rows, &mut self.costs);
        Simulation {
            rows: self.rows,
            costs: self.costs,
            current,
            elapsed_days: self.elapsed_days,
        }
    }
}

/// 一次回測的各期結果與目前持股。
pub(super) struct Simulation {
    /// 已滿持有天數的各期持股與報酬。
    pub(super) rows: Vec<PeriodRow>,
    /// 與 `rows` 一一對應的換手與扣成本後報酬。
    pub(super) costs: Vec<PeriodCost>,
    /// 最後一次換股、還沒滿持有天數的持股（見 [`split_current`]）。
    pub(super) current: Option<(PeriodRow, PeriodCost)>,
    /// `current` 換股日之後到迄日經過的交易日數。
    pub(super) elapsed_days: Option<i64>,
}

impl<'a> Backtest<'a> {
    /// 驗證參數、確認產業存在，並決定實際的起訖日；失敗時回錯誤回應（422／404／500）。
    pub(super) async fn prepare(params: &'a ScreenBacktestParams) -> Result<Self, Box<Response>> {
        let unprocessable =
            |message: &str| Box::new(error_response(StatusCode::UNPROCESSABLE_ENTITY, message));
        let (from, to) = match (
            parse_optional_date(params.from.as_deref()),
            parse_optional_date(params.to.as_deref()),
        ) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(message), _) | (_, Err(message)) => return Err(unprocessable(message)),
        };
        let hold_days = params.hold_days.unwrap_or(20);
        if !HOLD_DAYS.contains(&hold_days) {
            return Err(unprocessable("hold_days 必須為 5、20、60、120 或 240"));
        }
        let limit = params.limit.unwrap_or(20);
        if !(1..=100).contains(&limit) {
            return Err(unprocessable("limit 必須介於 1 至 100"));
        }
        if params
            .market_filter_ma
            .is_some_and(|days| !MARKET_FILTER_DAYS.contains(&days))
        {
            return Err(unprocessable("market_filter_ma 必須為 20、60、120 或 240"));
        }
        if params
            .market_filter_breadth
            .is_some_and(|percent| !(1..=99).contains(&percent))
        {
            return Err(unprocessable("market_filter_breadth 必須介於 1 至 99"));
        }
        if params
            .market_filter_max_pe
            .is_some_and(|pe| !(pe.is_finite() && (5.0..=100.0).contains(&pe)))
        {
            return Err(unprocessable("market_filter_max_pe 必須介於 5 至 100"));
        }
        if params
            .market_filter_pe_percentile
            .is_some_and(|percentile| !(1..=99).contains(&percentile))
        {
            return Err(unprocessable(
                "market_filter_pe_percentile 必須介於 1 至 99",
            ));
        }
        if params
            .hold_min_rs_score
            .is_some_and(|score| !(1..=99).contains(&score))
        {
            return Err(unprocessable("hold_min_rs_score 必須介於 1 至 99"));
        }
        if params
            .max_per_industry
            .is_some_and(|count| !(1..=100).contains(&count))
        {
            return Err(unprocessable("max_per_industry 必須介於 1 至 100"));
        }
        if params
            .min_avg_trade_value
            .is_some_and(|value| !(1..=MAX_TRADE_VALUE_MILLION).contains(&value))
        {
            return Err(unprocessable(
                "min_avg_trade_value 必須介於 1 至 100000（百萬元）",
            ));
        }
        let rank_by = params.rank_by.as_deref().unwrap_or(RANK_BY[0]);
        if !RANK_BY.contains(&rank_by) {
            return Err(unprocessable(
                "rank_by 必須為 rs_score、trade_value、foreign_streak、trust_streak、revenue_yoy、operating_margin、pe_ratio 或 pb_ratio",
            ));
        }
        let Some(weighting) = Weighting::parse(params.weighting.as_deref().unwrap_or("equal"))
        else {
            return Err(unprocessable(
                "weighting 必須為 equal、trade_value 或 sqrt_trade_value",
            ));
        };
        if params
            .max_weight
            .is_some_and(|percent| !(1..=100).contains(&percent))
        {
            return Err(unprocessable("max_weight 必須介於 1 至 100"));
        }
        if params
            .stop_loss
            .is_some_and(|percent| !(1..=50).contains(&percent))
        {
            return Err(unprocessable("stop_loss 必須介於 1 至 50"));
        }
        if params
            .trailing_stop
            .is_some_and(|percent| !(1..=50).contains(&percent))
        {
            return Err(unprocessable("trailing_stop 必須介於 1 至 50"));
        }
        let etf_weight = params.etf_weight.unwrap_or(0);
        if etf_weight > 100 {
            return Err(unprocessable("etf_weight 必須介於 0 至 100"));
        }
        let keep_rank = params.keep_rank.unwrap_or(limit);
        if !(limit..=MAX_KEEP_RANK).contains(&keep_rank) {
            return Err(unprocessable("keep_rank 必須介於 limit 至 200"));
        }
        let fee_discount = params.fee_discount.unwrap_or(1.0);
        if !(0.0..=1.0).contains(&fee_discount) {
            return Err(unprocessable("fee_discount 必須介於 0 至 1"));
        }
        let Some(execution) = ExecutionPrice::parse(params.execution_price.as_deref()) else {
            return Err(unprocessable("execution_price 必須為 close 或 next_open"));
        };
        if execution == ExecutionPrice::NextOpen
            && (params.stop_loss.is_some() || params.trailing_stop.is_some())
        {
            return Err(unprocessable(
                "execution_price=next_open 不可與 stop_loss／trailing_stop 併用",
            ));
        }
        let slippage_bps = params.slippage_bps.unwrap_or(0);
        if slippage_bps > MAX_SLIPPAGE_BPS {
            return Err(unprocessable("slippage_bps 必須介於 0 至 200"));
        }
        let market = params.market.as_deref().unwrap_or("all");
        let Some(market_id) = market_id_for_stocks(market) else {
            return Err(unprocessable("market 必須為 all、twse 或 tpex"));
        };
        if let Some(response) =
            signal_filter_error(params.kind.as_deref(), params.indicator.as_deref())
        {
            return Err(Box::new(response));
        }
        if let Err(message) = validate_rs_range(params.min_rs_score, params.max_rs_score) {
            return Err(unprocessable(message));
        }
        if let Err(message) = validate_streaks(params.min_foreign_streak, params.min_trust_streak) {
            return Err(unprocessable(message));
        }
        if let Err(message) = validate_fundamentals(params) {
            return Err(unprocessable(message));
        }
        if let Some(response) = industry_error(params.industry.as_deref()).await {
            return Err(Box::new(response));
        }
        let excluded =
            parse_excluded_industries(params.exclude_industry.as_deref()).map_err(unprocessable)?;
        if params
            .industry
            .as_ref()
            .is_some_and(|industry| excluded.contains(industry))
        {
            return Err(unprocessable(
                "exclude_industry 不可包含 industry 指定的產業",
            ));
        }
        for industry in &excluded {
            if industry_error(Some(industry)).await.is_some() {
                return Err(unprocessable(
                    "exclude_industry 必須為產業名稱（逗號分隔），例如「金融保險業」",
                ));
            }
        }

        let latest: Option<NaiveDate> = sqlx::query_scalar(LATEST_RS_DATE_SQL)
            .bind(to)
            .fetch_one(database::get_connection())
            .await
            .map_err(|error| Box::new(database_error(error)))?;
        let Some(to) = latest else {
            return Err(Box::new(error_response(
                StatusCode::NOT_FOUND,
                "查無相對強度資料",
            )));
        };
        let from = resolve_backtest_period(from, to).map_err(unprocessable)?;
        let split = parse_optional_date(params.split.as_deref()).map_err(unprocessable)?;
        if split.is_some_and(|split| split <= from || split > to) {
            return Err(unprocessable("split 必須晚於起日、不晚於迄日"));
        }
        Ok(Self {
            params,
            from,
            to,
            market,
            market_id,
            hold_days,
            limit,
            keep_rank,
            fee_discount,
            execution,
            slippage_bps,
            rank_by,
            etf_weight,
            weighting,
            max_weight: params.max_weight,
            split,
            excluded_industries: excluded,
            stops: StopRule {
                stop_loss: params.stop_loss.map(f64::from),
                trailing_stop: params.trailing_stop.map(f64::from),
            },
            bars: BarCache::default(),
            daily_path: true,
        })
    }

    /// 偏移、前緣、滾動前推、歸因與策略通知一次跑多組回測，只用期末口徑的分布或持股：不載入日線
    /// 算日頻路徑（日線只在有停損時載入），避免每組都讀數十萬列日線。
    pub(super) fn period_end_only(mut self) -> Self {
        self.daily_path = false;
        self
    }

    /// 實際扣成本用的手續費折數（含滑價，見 [`cost_fee_discount`]）。
    pub(super) fn cost_fee_discount(&self) -> f64 {
        cost_fee_discount(self.fee_discount, self.slippage_bps)
    }

    /// 條件或排序用到外資／投信連續買超（要查籌碼）。
    pub(super) fn with_chip(&self) -> bool {
        self.params.min_foreign_streak.is_some()
            || self.params.min_trust_streak.is_some()
            || self.rank_by.ends_with("_streak")
    }

    /// 以換股日偏移 `rebalance_offset` 跑一次回測：混合 `etf_weight` 的 0050 核心部位，並把還沒滿
    /// 持有天數的最後一期移到 [`Simulation::current`]。
    pub(super) async fn simulate(&self, rebalance_offset: u16) -> Result<Simulation, sqlx::Error> {
        Ok(self.run(rebalance_offset).await?.finish(self.etf_weight))
    }

    /// 以換股日偏移 `rebalance_offset` 查候選股、決定每期持股並扣交易成本，尚未混合 0050。
    pub(super) async fn run(&self, rebalance_offset: u16) -> Result<Raw, sqlx::Error> {
        let candidates = self.candidates(rebalance_offset, self.keep_rank).await?;
        self.assemble(candidates, self.keep_rank, self.max_weight)
            .await
    }

    /// 以換股日偏移 `rebalance_offset` 查每期的候選股；每期至少讀到第 `keep_rank` 名（設
    /// `max_per_industry` 時讀到 [`MAX_KEEP_RANK`] 名，同產業額滿後要往後補）。
    pub(super) async fn candidates(
        &self,
        rebalance_offset: u16,
        keep_rank: u8,
    ) -> Result<Vec<CandidateRow>, sqlx::Error> {
        let params = self.params;
        let with_chip = self.with_chip();
        let mut query = sqlx::query_as(match (with_chip, self.execution) {
            (true, ExecutionPrice::Close) => BACKTEST_WITH_CHIP_SQL,
            (true, ExecutionPrice::NextOpen) => BACKTEST_WITH_CHIP_NEXT_OPEN_SQL,
            (false, ExecutionPrice::Close) => BACKTEST_SQL,
            (false, ExecutionPrice::NextOpen) => BACKTEST_NEXT_OPEN_SQL,
        })
        .bind(self.from)
        .bind(self.to)
        .bind(self.market_id)
        .bind(i32::from(self.hold_days))
        .bind(i64::from(if params.max_per_industry.is_some() {
            MAX_KEEP_RANK
        } else {
            keep_rank
        }))
        .bind(params.industry.as_deref())
        .bind(params.min_rs_score.map(i16::from))
        .bind(params.max_rs_score.map(i16::from))
        .bind(params.indicator.as_deref())
        .bind(params.kind.as_deref())
        .bind(params.market_filter_ma.map(i32::from))
        .bind(params.hold_min_rs_score.map(i16::from))
        .bind(params.market_filter_breadth.map(i32::from))
        .bind(i32::from(rebalance_offset))
        .bind(
            params
                .min_avg_trade_value
                .map(|value| i64::from(value) * 1_000_000),
        )
        .bind(self.rank_by)
        .bind(self.weighting.as_str())
        .bind((!self.excluded_industries.is_empty()).then_some(&self.excluded_industries))
        .bind(params.min_revenue_yoy_percent)
        .bind(params.min_revenue_yoy_streak.map(i16::from))
        .bind(params.min_revenue_yoy_3m_percent)
        .bind(params.min_eps)
        .bind(params.min_roe_percent)
        .bind(params.min_gross_margin_percent)
        .bind(params.min_operating_margin_percent)
        .bind(params.margins_yoy_up.unwrap_or(false))
        .bind(params.max_debt_ratio_percent)
        .bind(params.revenue_record.unwrap_or(false))
        .bind(params.min_monthly_revenue)
        .bind(params.positive_free_cash_flow.unwrap_or(false))
        .bind(params.max_pe)
        .bind(params.max_pb)
        .bind(params.market_filter_max_pe)
        .bind(params.market_filter_pe_percentile.map(i32::from));
        if with_chip {
            query = query
                .bind(params.min_foreign_streak.map(i64::from))
                .bind(params.min_trust_streak.map(i64::from));
        }
        // 回測主查詢依條件要 1～2 秒，偏移穩健度與前緣一次就跑 20 個偏移；用批次連線池（慢查詢門檻
        // 5 秒），避免每次回測都留下一串慢查詢警告，蓋過真正的退化。
        query.fetch_all(database::get_batch_connection()).await
    }

    /// 依候選股決定每期持股（續抱緩衝 `keep_rank`）、期間停損、持股權重（單檔上限 `max_weight`）並扣
    /// 交易成本，尚未混合 0050；`candidates` 每期至少要讀到第 `keep_rank` 名（見 [`Self::candidates`]）。
    pub(super) async fn assemble(
        &self,
        candidates: Vec<CandidateRow>,
        keep_rank: u8,
        max_weight: Option<u8>,
    ) -> Result<Raw, sqlx::Error> {
        let params = self.params;
        let elapsed_days = candidates
            .last()
            .filter(|row| row.pending)
            .map(|row| row.elapsed_days);
        let mut rows = select_holdings(
            candidates,
            usize::from(self.limit),
            usize::from(keep_rank),
            params.max_per_industry.map(usize::from),
        );
        let fee_discount = self.cost_fee_discount();
        // 停損要整段日線且各偏移共用快取；只算日頻路徑時只讀各持股的持有期間。次日開盤成交不支援
        // 停損（驗證已擋下），日線只用來算以開盤為端點的日頻路徑。
        let bars = match self.execution {
            ExecutionPrice::Close if self.stops.is_active() => {
                Some(self.bars.load(&rows, self.from, self.to).await?)
            }
            _ if self.daily_path => Some(load_holding_windows(&rows).await?),
            _ => None,
        };
        if let Some(bars) = bars
            .as_ref()
            .filter(|_| self.execution == ExecutionPrice::Close)
        {
            apply_stops(&mut rows, bars, self.stops, self.hold_days);
        }
        weigh(
            &mut rows,
            self.weighting,
            max_weight.map(|percent| f64::from(percent) / 100.0),
        );
        let costs = trading_costs(&rows, fee_discount);
        if let Some(bars) = bars.as_ref().filter(|_| self.daily_path) {
            let ratios = match self.execution {
                ExecutionPrice::NextOpen => Some(load_open_ratios(&rows, bars).await?),
                ExecutionPrice::Close => None,
            };
            populate_daily_returns(
                &mut rows,
                bars,
                self.stops,
                fee_discount,
                &costs,
                ratios.as_ref(),
            );
        }
        Ok(Raw {
            rows,
            costs,
            elapsed_days,
            fee_discount,
        })
    }
}
