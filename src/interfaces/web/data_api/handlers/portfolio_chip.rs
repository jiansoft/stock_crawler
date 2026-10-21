//! `/portfolio/chip` handler：持股籌碼日報。
//!
//! 某交易日收盤時仍持有的普通股，當天的三大法人買賣超、融資融券增減與主力進出（另附近
//! [`BROKER_RECENT_DAYS`] 個主力交易日的累計），並依 Telegram 通知的門檻標出達標原因
//! （[`crate::domain::portfolio::chip`]）。屬個人持股資料。

use std::collections::HashMap;

use anyhow::Result;
use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use rust_decimal::{Decimal, prelude::ToPrimitive};

use super::{error_response, parse_optional_date, repository_error, round2};
use crate::domain::chip::BrokerNetRecord;
use crate::domain::portfolio::chip::{
    BrokerPeriodNet, ChipTradingDay, HoldingBrokerQuery, HoldingChip, HoldingChipQuery,
};
use crate::infra::database::repository::portfolio::PgPortfolioRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PortfolioChipBroker, PortfolioChipBrokerNet, PortfolioChipBrokerRecent,
    PortfolioChipItem, PortfolioChipParams, PortfolioChipResponse,
};

/// 主力累計涵蓋的交易日數（含當天），與 20:40 主力進出通知相同。
const BROKER_RECENT_DAYS: usize = 5;

/// 主力累計：實際涵蓋的交易日數與各股合計。
type BrokerRecent = (usize, Vec<BrokerPeriodNet>);

/// 查詢持股籌碼日報。
///
/// # Errors
///
/// 日期格式錯誤回 422、驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。查詢日之前沒有任何籌碼
/// 資料時回 200 與空清單（`date` 為 `null`）。
#[utoipa::path(get, path = "/api/v1/portfolio/chip", tag = "data-api", params(PortfolioChipParams), responses((status = 200, body = PortfolioChipResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn portfolio_chip(Query(params): Query<PortfolioChipParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let repository = PgPortfolioRepository::new();
    let day = match repository.resolve_chip_day(date).await {
        Ok(day) => day,
        Err(error) => return repository_error(error),
    };
    let chips = match day {
        Some(day) => match repository.fetch_holding_chips(day.date).await {
            Ok(chips) => chips,
            Err(error) => return repository_error(error),
        },
        None => Vec::new(),
    };
    let recent = match day {
        Some(day) => match broker_recent(&repository, day.date, &chips).await {
            Ok(recent) => recent,
            Err(error) => return repository_error(error),
        },
        None => (0, Vec::new()),
    };
    Json(response(day, chips, recent)).into_response()
}

/// `date`（含）之前最近 [`BROKER_RECENT_DAYS`] 個有主力資料的交易日裡各持股的主力合計；
/// 沒有任何主力資料時為 `(0, [])`。
async fn broker_recent(
    repository: &impl HoldingBrokerQuery,
    date: NaiveDate,
    chips: &[HoldingChip],
) -> Result<BrokerRecent> {
    let dates = repository
        .fetch_broker_dates(date, BROKER_RECENT_DAYS)
        .await?;
    let Some(&from) = dates.last() else {
        return Ok((0, Vec::new()));
    };
    let symbols: Vec<String> = chips.iter().map(|chip| chip.stock_symbol.clone()).collect();
    let nets = repository.fetch_broker_period(from, date, &symbols).await?;
    Ok((dates.len(), nets))
}

/// 組出回應：股換算成張、佔量取兩位小數、達標原因用代碼、附主力近日累計。
fn response(
    day: Option<ChipTradingDay>,
    chips: Vec<HoldingChip>,
    (recent_days, recent): BrokerRecent,
) -> PortfolioChipResponse {
    let recent: HashMap<String, BrokerPeriodNet> = recent
        .into_iter()
        .map(|net| (net.stock_symbol.clone(), net))
        .collect();
    let items: Vec<PortfolioChipItem> = chips
        .iter()
        .map(|chip| PortfolioChipItem {
            broker_recent: recent
                .get(&chip.stock_symbol)
                .map(|net| PortfolioChipBrokerRecent {
                    main_net: net.main_net,
                    buy_days: net.buy_days,
                    sell_days: net.sell_days,
                }),
            ..item(chip)
        })
        .collect();
    PortfolioChipResponse {
        date: day.map(|day| day.date.to_string()),
        previous_date: day
            .and_then(|day| day.previous)
            .map(|date| date.to_string()),
        next_date: day.and_then(|day| day.next).map(|date| date.to_string()),
        notable: items.iter().filter(|item| !item.reasons.is_empty()).count(),
        broker_recent_days: recent_days,
        items,
    }
}

fn item(chip: &HoldingChip) -> PortfolioChipItem {
    PortfolioChipItem {
        stock_symbol: chip.stock_symbol.clone(),
        name: chip.name.clone(),
        closing_price: chip.closing_price.and_then(|price| price.to_f64()),
        volume: chip.volume.map(lots),
        foreign_net: chip.foreign_net.map(lots),
        trust_net: chip.trust_net.map(lots),
        dealer_net: chip.dealer_net.map(lots),
        foreign_share: share_of_volume(chip.foreign_net, chip.volume),
        trust_share: share_of_volume(chip.trust_net, chip.volume),
        foreign_streak: chip.foreign_streak.unwrap_or(0),
        trust_streak: chip.trust_streak.unwrap_or(0),
        margin_balance: chip.margin_balance,
        margin_change: difference(chip.margin_balance, chip.margin_previous),
        short_balance: chip.short_balance,
        short_change: difference(chip.short_balance, chip.short_previous),
        broker: chip.broker.as_ref().map(|broker| PortfolioChipBroker {
            main_net: broker.main_net(),
            main_share: decimal(broker.main_share),
            buyers: broker.buyers.iter().map(broker_net).collect(),
            sellers: broker.sellers.iter().map(broker_net).collect(),
        }),
        broker_recent: None,
        reasons: chip
            .reasons()
            .into_iter()
            .map(|reason| reason.code().to_owned())
            .collect(),
    }
}

fn broker_net(record: &BrokerNetRecord) -> PortfolioChipBrokerNet {
    PortfolioChipBrokerNet {
        name: record.name.clone(),
        buy: record.buy,
        sell: record.sell,
        net: record.net,
        share: decimal(record.share),
    }
}

/// 股數四捨五入成張數（遠離零）。
fn lots(shares: i64) -> i64 {
    (shares as f64 / 1_000.0).round() as i64
}

/// 買賣超佔成交量的百分比（帶正負號，兩位小數）；沒有成交量時為 `None`。
fn share_of_volume(net: Option<i64>, volume: Option<i64>) -> Option<f64> {
    let (net, volume) = (net?, volume?);
    (volume > 0).then(|| round2(net as f64 * 100.0 / volume as f64))
}

/// 今日減前日；任一方缺資料時為 `None`。
fn difference(today: Option<i64>, previous: Option<i64>) -> Option<i64> {
    Some(today? - previous?)
}

fn decimal(value: Decimal) -> f64 {
    round2(value.to_f64().unwrap_or_default())
}

#[cfg(test)]
mod tests;
