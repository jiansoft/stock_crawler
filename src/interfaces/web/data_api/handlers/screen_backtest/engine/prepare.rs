//! 回測參數驗證：檢查範圍、確認產業存在，並決定實際起訖日。

use axum::{http::StatusCode, response::Response};
use chrono::NaiveDate;

use super::super::portfolio::Weighting;
use super::super::sql::{FIRST_HOLDER_WEEK_SQL, LATEST_RS_DATE_SQL};
use super::super::stops::{BarCache, StopRule};
use super::super::validate::{
    parse_excluded_industries, resolve_backtest_period, validate_fundamentals,
    validate_major_change_period,
};
use super::super::{
    HOLD_DAYS, MARKET_FILTER_DAYS, MAX_KEEP_RANK, MAX_TRADE_VALUE_MILLION, RANK_BY,
};
use super::{Backtest, ExecutionPrice, MAX_SLIPPAGE_BPS};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::ScreenBacktestParams;
use crate::interfaces::web::data_api::handlers::signal_history::{
    validate_rs_range, validate_streaks,
};
use crate::interfaces::web::data_api::handlers::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    signal_filter_error,
};

impl<'a> Backtest<'a> {
    /// 驗證參數、確認產業存在，並決定實際的起訖日；失敗時回錯誤回應（422／404／500）。
    pub(in super::super) async fn prepare(
        params: &'a ScreenBacktestParams,
    ) -> Result<Self, Box<Response>> {
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
        let first_week: Option<NaiveDate> = match params.min_major_change {
            Some(_) => sqlx::query_scalar(FIRST_HOLDER_WEEK_SQL)
                .fetch_one(database::get_connection())
                .await
                .map_err(|error| Box::new(database_error(error)))?,
            None => None,
        };
        validate_major_change_period(params, from, first_week)
            .map_err(|message| unprocessable(&message))?;
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
}
