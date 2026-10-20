//! # 策略換股通知
//!
//! 每個交易日 22:10（籌碼 21:50 入庫之後），對每個 `alert` 為 true 的具名策略（`strategy_preset`，
//! 條件與 `/stocks/screen/backtest` 的 query string 相同，必須指定 `from`，換股日從起日起算）各跑一次
//! 回測；最後一次換股是今天時，用 Telegram 通知新買進、續抱與賣出的股票與權重，並對照目前的實際持股
//! （策略持股中已持有、尚未持有的股票，策略賣出但仍持有的股票，以及 0050 核心部位）。`.env` 的
//! [`QUERY_ENV`]／[`NAME_ENV`] 仍可另設一組（與具名策略條件相同時不重複通知）。都沒有就不做事；
//! 讀不到實際持股時只略過對照；某個策略失敗時其餘照常通知，最後回傳錯誤。
//!
//! 換股日另把當天的持股寫進 `strategy_rebalance`（見 [`rebalance_record`]），供
//! `/strategy-presets/{name}/track` 追蹤實際表現；寫入失敗只記 warning，通知照發。通知同時附上依換股
//! 紀錄結算的上一期實盤表現與追蹤以來的累積（vs 0050）；還沒有上一期紀錄或結算失敗時不附。策略持股有
//! 基本面警示（營收連續年減、三率三降、營業虧損，見 [`crate::domain::portfolio::health`]）時另列一行，
//! 查詢失敗只略過。
//!
//! 回測引擎在 `interfaces::web::data_api`，這裡只透過 [`ports::StrategyGateway`] 取得換股結果。

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDate};

use self::message::build_message;
use super::holdings;
use crate::{
    app::ports::{self, StrategyGateway, StrategyRebalance},
    core::alert,
    domain::{
        portfolio::health::HoldingFundamentalsQuery,
        strategy::{
            StrategyPreset, StrategyPresetRepository,
            rebalance::{RecordedHolding, StrategyRebalanceRecord, StrategyRebalanceRepository},
        },
    },
    infra::database::repository::{
        portfolio::PgPortfolioRepository, strategy_preset::PgStrategyPresetRepository,
        strategy_rebalance::PgStrategyRebalanceRepository,
    },
};

mod message;

/// 另一組策略條件的環境變數（條件選股回測的 query string）；沒有設定或空白時只看具名策略。
pub const QUERY_ENV: &str = "STRATEGY_ALERT_QUERY";

/// 通知標題中的策略名稱環境變數；未設定時只寫「策略換股」。
pub const NAME_ENV: &str = "STRATEGY_ALERT_NAME";

/// 要通知的一個策略：名稱（通知標題用，可為空）與條件。
#[derive(Debug, Clone, PartialEq, Eq)]
struct AlertStrategy {
    name: String,
    query: String,
}

/// 要通知的策略：`alert` 為 true 的具名策略（依名稱），再加上環境變數的那一組（條件空白或與某個
/// 具名策略相同時不加）。
fn alert_strategies(
    presets: Vec<StrategyPreset>,
    env_query: Option<String>,
    env_name: Option<String>,
) -> Vec<AlertStrategy> {
    let mut strategies: Vec<AlertStrategy> = presets
        .into_iter()
        .filter(|preset| preset.alert)
        .map(|preset| AlertStrategy {
            name: preset.name,
            query: preset.query.trim().to_owned(),
        })
        .collect();
    let query = env_query.unwrap_or_default().trim().to_owned();
    if !query.is_empty() && !strategies.iter().any(|strategy| strategy.query == query) {
        strategies.push(AlertStrategy {
            name: env_name.unwrap_or_default().trim().to_owned(),
            query,
        });
    }
    strategies
}

/// 排程入口：對每個要通知的策略，今天是換股日時發通知。
///
/// # Errors
///
/// 讀不到具名策略、策略 gateway 未註冊，或有策略的回測條件錯誤或回測失敗時回傳錯誤（其餘策略照常
/// 通知）。
pub async fn execute() -> Result<()> {
    let presets = PgStrategyPresetRepository::new()
        .list()
        .await
        .context("讀取具名策略失敗")?;
    let strategies = alert_strategies(
        presets,
        std::env::var(QUERY_ENV).ok(),
        std::env::var(NAME_ENV).ok(),
    );
    if strategies.is_empty() {
        tracing::info!("策略換股通知：沒有要通知的具名策略，也未設定 {QUERY_ENV}，略過");
        return Ok(());
    }
    let gateway = ports::strategy_gateway()?;
    notify(
        gateway.as_ref(),
        &PgStrategyRebalanceRepository::new(),
        &PgPortfolioRepository::new(),
        &strategies,
        Local::now().date_naive(),
    )
    .await
}

/// 手動回補用：以具名策略目前的條件跑一次回測，把最後一次換股（還沒滿持有天數的那一期）寫進換股
/// 紀錄，回傳換股日與是否新寫入；策略不存在回錯誤，最後一期剛好滿持有天數回 `None`。
///
/// 換股通知啟用前的那一期沒有被記錄，用這個補上，之後的換股由 22:10 的通知照常記錄。
///
/// # Errors
///
/// 讀不到策略、策略 gateway 未註冊、回測失敗或寫入失敗時回傳錯誤。
pub async fn record_current_rebalance(preset_name: &str) -> Result<Option<(NaiveDate, bool)>> {
    let preset = PgStrategyPresetRepository::new()
        .find(preset_name)
        .await
        .context("讀取具名策略失敗")?
        .with_context(|| format!("查無具名策略：{preset_name}"))?;
    let strategy = AlertStrategy {
        name: preset.name,
        query: preset.query.trim().to_owned(),
    };
    let Some(rebalance) = ports::strategy_gateway()?
        .latest_rebalance(&strategy.query)
        .await?
    else {
        return Ok(None);
    };
    let record = rebalance_record(&strategy, &rebalance);
    let inserted = PgStrategyRebalanceRepository::new().save(&record).await?;
    Ok(Some((record.date, inserted)))
}

/// 逐一跑策略回測，今天換股的寫入換股紀錄並發通知；實際持股只在第一次需要時讀一次。
async fn notify(
    gateway: &dyn StrategyGateway,
    recorder: &dyn StrategyRebalanceRepository,
    fundamentals: &dyn HoldingFundamentalsQuery,
    strategies: &[AlertStrategy],
    today: NaiveDate,
) -> Result<()> {
    let mut held: Option<Option<BTreeSet<String>>> = None;
    let mut failures: Vec<String> = Vec::new();
    for strategy in strategies {
        let rebalance = match rebalance_today(gateway, &strategy.query, today).await {
            Ok(Some(rebalance)) => rebalance,
            Ok(None) => continue,
            Err(error) => {
                tracing::error!("策略換股通知：{} 失敗：{error:?}", strategy.name);
                failures.push(format!("{}：{error:#}", strategy.name));
                continue;
            }
        };
        let record = rebalance_record(strategy, &rebalance);
        match recorder.save(&record).await {
            Ok(true) => tracing::info!(
                "策略換股通知：已記錄 {} {} 的換股",
                record.preset_name,
                record.date
            ),
            Ok(false) => tracing::info!(
                "策略換股通知：{} {} 已有換股紀錄，保留原本的",
                record.preset_name,
                record.date
            ),
            Err(error) => tracing::warn!(
                "策略換股通知：寫入 {} 換股紀錄失敗：{error:?}",
                record.preset_name
            ),
        }
        let settlement = match gateway.settlement(&record.preset_name, record.date).await {
            Ok(settlement) => settlement,
            Err(error) => {
                tracing::warn!(
                    "策略換股通知：結算 {} 上一期實盤失敗，略過：{error:?}",
                    record.preset_name
                );
                None
            }
        };
        if held.is_none() {
            held = Some(match holdings::active_holding_symbols().await {
                Ok(held) => Some(held),
                Err(error) => {
                    tracing::warn!("策略換股通知：讀取實際持股失敗，略過對照：{error:?}");
                    None
                }
            });
        }
        let actual = held.as_ref().and_then(Option::as_ref);
        let symbols: Vec<String> = rebalance
            .added
            .iter()
            .chain(&rebalance.kept)
            .map(|holding| holding.symbol.clone())
            .collect();
        let warned = fundamentals
            .fetch_holding_fundamentals(&symbols)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!("策略換股通知：讀取持股基本面失敗，略過警示：{error:?}");
                Vec::new()
            });
        alert::send_message(&build_message(
            &rebalance,
            &strategy.name,
            actual,
            settlement.as_ref(),
            &warned,
        ))
        .await;
    }
    if !failures.is_empty() {
        bail!("策略換股通知失敗：{}", failures.join("；"));
    }
    Ok(())
}

/// 沒有名稱（只設 [`QUERY_ENV`]）的策略在換股紀錄中的名稱。
const UNNAMED_STRATEGY: &str = "STRATEGY_ALERT_QUERY";

/// 換股紀錄：策略部位的新買進（在前）與續抱持股及權重；賣出的不記。
fn rebalance_record(
    strategy: &AlertStrategy,
    rebalance: &StrategyRebalance,
) -> StrategyRebalanceRecord {
    StrategyRebalanceRecord {
        preset_name: if strategy.name.is_empty() {
            UNNAMED_STRATEGY.to_owned()
        } else {
            strategy.name.clone()
        },
        date: rebalance.date,
        query: strategy.query.clone(),
        hold_days: rebalance.hold_days,
        etf_weight: rebalance.etf_weight,
        cash: rebalance.cash,
        market_filtered: rebalance.market_filtered,
        holdings: rebalance
            .added
            .iter()
            .chain(&rebalance.kept)
            .map(|holding| RecordedHolding {
                symbol: holding.symbol.clone(),
                name: holding.name.clone(),
                weight: holding.weight,
            })
            .collect(),
    }
}

/// 跑一次策略回測，取今天的換股；最後一次換股不是 `today`（今天不是換股日或休市）時回 `None`。
async fn rebalance_today(
    gateway: &dyn StrategyGateway,
    query: &str,
    today: NaiveDate,
) -> Result<Option<StrategyRebalance>> {
    let Some(rebalance) = gateway.latest_rebalance(query).await? else {
        tracing::info!("策略換股通知：最後一期已滿持有天數，今天不是換股日，略過");
        return Ok(None);
    };
    if rebalance.date != today {
        tracing::info!(
            "策略換股通知：最後一次換股為 {}，不是今天 {today}，略過",
            rebalance.date
        );
        return Ok(None);
    }
    Ok(Some(rebalance))
}

#[cfg(test)]
pub(in crate::app::event::taiwan_stock) mod tests;
