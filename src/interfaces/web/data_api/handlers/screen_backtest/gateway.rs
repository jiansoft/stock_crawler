//! 策略換股通知的 adapter：實作 [`StrategyGateway`]，以設定的回測條件跑一次回測（與
//! `/stocks/screen/backtest` 相同的驗證與計算），取最後一次換股、還沒滿持有天數的持股。

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use axum::{extract::Query, http::Uri, response::Response};
use chrono::NaiveDate;

use super::{Backtest, Simulation, current_holdings, exited, returns_to_date, stock_names};
use crate::app::ports::{StrategyGateway, StrategyHolding, StrategyRebalance};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::ScreenBacktestParams;

/// 讀取錯誤回應內容的上限（位元組）。
const ERROR_BODY_LIMIT: usize = 64 * 1024;

/// 該交易日的籌碼（含連續買超天數）是否已入庫。
const CHIP_READY_SQL: &str = r#"SELECT EXISTS (SELECT 1 FROM chip_daily WHERE "date" = $1)"#;

/// 以條件選股回測引擎計算策略換股的 gateway；由 `main` 註冊到 [`crate::app::ports`]。
pub struct BacktestStrategyGateway;

#[async_trait]
impl StrategyGateway for BacktestStrategyGateway {
    async fn latest_rebalance(&self, query: &str) -> Result<Option<StrategyRebalance>> {
        let uri: Uri = format!("/?{query}")
            .parse()
            .context("策略條件不是有效的 query string")?;
        let Query(params) = Query::<ScreenBacktestParams>::try_from_uri(&uri)
            .map_err(|error| anyhow!("策略條件無法解析：{error}"))?;
        if params.from.is_none() {
            bail!("策略條件必須指定 from（換股日從起日起算，省略時每天都會變）");
        }
        let backtest = match Backtest::prepare(&params).await {
            Ok(backtest) => backtest,
            Err(response) => bail!("策略條件不正確：{}", rejection(*response).await),
        };
        let offset = params.rebalance_offset.unwrap_or(0);
        if offset >= backtest.hold_days {
            bail!("策略條件不正確：rebalance_offset 必須介於 0 至 hold_days − 1");
        }
        let Simulation {
            rows,
            current,
            elapsed_days,
            ..
        } = backtest
            .simulate(offset)
            .await
            .context("策略回測查詢失敗")?;
        let Some((row, cost)) = current else {
            return Ok(None);
        };
        if backtest.with_chip() && !chip_ready(row.date).await? {
            bail!(
                "{} 的籌碼資料尚未入庫，無法判斷外資／投信連續買超",
                row.date
            );
        }
        let previous = rows.last();
        let exits = exited(&row, previous);
        let symbols: Vec<&str> = row
            .symbols
            .iter()
            .map(String::as_str)
            .chain(exits.iter().map(|(symbol, _)| symbol.as_str()))
            .collect();
        let names = stock_names(&symbols).await.context("讀取股票名稱失敗")?;
        let date = row.date;
        let returns = returns_to_date(&row, backtest.to, elapsed_days.unwrap_or(0)).await;
        let current = current_holdings(
            (row, cost),
            previous,
            elapsed_days.unwrap_or(0),
            backtest.hold_days,
            &names,
            &returns,
        );
        let (added, kept): (Vec<_>, Vec<_>) = current
            .holdings
            .into_iter()
            .partition(|holding| holding.added);
        let holding = |symbol: String, name: Option<String>, weight: f64| StrategyHolding {
            symbol,
            name,
            weight,
        };
        Ok(Some(StrategyRebalance {
            date,
            hold_days: backtest.hold_days,
            etf_weight: backtest.etf_weight,
            market_filtered: current.market_filtered,
            cash: current.cash,
            turnover: current.turnover,
            added: added
                .into_iter()
                .map(|item| holding(item.symbol, item.name, item.weight))
                .collect(),
            kept: kept
                .into_iter()
                .map(|item| holding(item.symbol, item.name, item.weight))
                .collect(),
            exited: current
                .exited
                .into_iter()
                .map(|item| holding(item.symbol, item.name, item.weight))
                .collect(),
        }))
    }
}

/// `date` 的籌碼是否已入庫。
async fn chip_ready(date: NaiveDate) -> Result<bool> {
    sqlx::query_scalar(CHIP_READY_SQL)
        .bind(date)
        .fetch_one(database::get_connection())
        .await
        .context("檢查籌碼資料失敗")
}

/// 驗證失敗的回應：狀態碼與錯誤訊息。
async fn rejection(response: Response) -> String {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), ERROR_BODY_LIMIT)
        .await
        .unwrap_or_default();
    let message = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|json| json["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| String::from_utf8_lossy(&body).into_owned());
    format!("{message}（HTTP {}）", status.as_u16())
}
