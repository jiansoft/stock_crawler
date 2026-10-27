//! 每週市場摘要的策略一段：有設定策略換股通知的條件（見 [`super::super::strategy_alert`]）時，列出
//! 這一期換股日、本期至今的報酬與 0050 對照，以及表現最好、最差的幾檔持股；另附最近一次策略優勢滾動
//! 檢查（見 [`super::super::strategy_edge`]）的近期與全期數字。

use super::TOP;
use crate::app::{
    event::taiwan_stock::{
        strategy_alert::{AlertStrategy, NAME_ENV, QUERY_ENV, alert_strategies},
        strategy_edge::{self, EdgeRecord},
    },
    ports::{self, StrategyGateway, StrategyHolding, StrategyRebalance, StrategySettlement},
};

/// 所有策略的持股段落與最近一次優勢檢查；個別失敗不影響其餘段落。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StrategySection {
    pub(super) positions: Vec<StrategyPosition>,
    pub(super) edges: Vec<EdgeRecord>,
}

/// 單一策略的當期回測與依換股紀錄計算的實盤累積。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StrategyPosition {
    pub(super) name: String,
    pub(super) rebalance: StrategyRebalance,
    pub(super) settlement: Option<StrategySettlement>,
}

/// 讀取具名策略與環境變數條件；資料來源失敗只略過持股段，仍保留優勢段。
pub(super) async fn strategy_section() -> Option<StrategySection> {
    use crate::domain::strategy::StrategyPresetRepository;
    use crate::infra::database::repository::strategy_preset::PgStrategyPresetRepository;
    let edges = strategy_edge::load_latest().await;
    let presets = PgStrategyPresetRepository::new()
        .list()
        .await
        .unwrap_or_else(|error| {
            tracing::warn!("每週市場摘要：讀取具名策略失敗：{error:#}");
            Vec::new()
        });
    let strategies = alert_strategies(
        presets,
        std::env::var(QUERY_ENV).ok(),
        std::env::var(NAME_ENV).ok(),
    );
    let positions = if strategies.is_empty() {
        Vec::new()
    } else {
        match ports::strategy_gateway() {
            Ok(gateway) => {
                load_positions(
                    gateway.as_ref(),
                    &strategies,
                    chrono::Local::now().date_naive(),
                )
                .await
            }
            Err(error) => {
                tracing::warn!("每週市場摘要：策略 gateway 無法使用：{error:#}");
                Vec::new()
            }
        }
    };
    (!positions.is_empty() || !edges.is_empty()).then_some(StrategySection { positions, edges })
}

/// 逐一取得策略持股與截至報告日的實盤；回測失敗略過該段，結算失敗僅略過累積行。
async fn load_positions(
    gateway: &dyn StrategyGateway,
    strategies: &[AlertStrategy],
    today: chrono::NaiveDate,
) -> Vec<StrategyPosition> {
    let mut positions = Vec::new();
    for strategy in strategies {
        let rebalance = match gateway.latest_rebalance(&strategy.query).await {
            Ok(Some(rebalance)) => rebalance,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!("每週市場摘要：{} 回測失敗：{error:#}", strategy.name);
                continue;
            }
        };
        // settlement 只讀取 date 之前的紀錄；加一天納入報告日當天的換股，行情仍封頂於今天。
        let settlement = match gateway
            .settlement(&strategy.name, today.succ_opt().unwrap_or(today))
            .await
        {
            Ok(settlement) => settlement,
            Err(error) => {
                tracing::warn!("每週市場摘要：{} 實盤結算失敗：{error:#}", strategy.name);
                None
            }
        };
        positions.push(StrategyPosition {
            name: strategy.name.clone(),
            rebalance,
            settlement,
        });
    }
    positions
}

/// 報酬文字（帶正負號、兩位小數）；沒有為「—」。
fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_owned(), |value| format!("{value:+.2}%"))
}

/// 一檔持股：代號、名稱與本期至今報酬。
fn holding_text(holding: &StrategyHolding) -> String {
    let name = holding
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .map(|name| format!(" {name}"))
        .unwrap_or_default();
    format!(
        "{}{name} {}",
        holding.symbol,
        percent(holding.return_to_date)
    )
}

/// 策略一段的各行（開頭為空行）：換股一段，接著策略優勢檢查一段。
pub(super) fn lines(section: &StrategySection) -> Vec<String> {
    let mut lines = Vec::new();
    for position in &section.positions {
        lines.extend(rebalance_lines(&position.name, &position.rebalance));
        if let Some(settlement) = &position.settlement {
            lines.push(format!(
                "• 實盤追蹤 {}～{}（{} 期，未扣成本）：累積 {}，0050 {}",
                settlement.tracked_from,
                settlement.end,
                settlement.periods,
                percent(settlement.cumulative_return),
                percent(settlement.etf_cumulative_return)
            ));
        }
    }
    lines.extend(strategy_edge::weekly_lines(&section.edges));
    lines
}

/// 換股一段的各行（開頭為空行）。
fn rebalance_lines(name: &str, rebalance: &StrategyRebalance) -> Vec<String> {
    let title = if name.is_empty() {
        "策略持股".to_owned()
    } else {
        format!("策略持股（{name}）")
    };
    let remaining = (i64::from(rebalance.hold_days) - rebalance.days_elapsed).max(0);
    let mut lines = vec![
        String::new(),
        format!(
            "{title}：{} 換股、已過 {} 個交易日，再 {remaining} 個交易日換股",
            rebalance.date, rebalance.days_elapsed
        ),
    ];
    let mut held: Vec<&StrategyHolding> = rebalance.added.iter().chain(&rebalance.kept).collect();
    if rebalance.market_filtered || held.is_empty() {
        let reason = if rebalance.market_filtered {
            "大盤濾網沒過"
        } else {
            "沒有符合條件的股票"
        };
        lines.push(format!(
            "• {reason}，策略部位空手；0050 同期 {}",
            percent(rebalance.etf_return_to_date)
        ));
        return lines;
    }
    lines.push(format!(
        "• 持有 {} 檔，本期至今 {}（0050 {}，未扣成本）",
        held.len(),
        percent(rebalance.return_to_date),
        percent(rebalance.etf_return_to_date)
    ));
    held.retain(|holding| holding.return_to_date.is_some());
    held.sort_by(|a, b| {
        b.return_to_date
            .unwrap_or_default()
            .total_cmp(&a.return_to_date.unwrap_or_default())
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    if held.is_empty() {
        return lines;
    }
    let best: Vec<String> = held.iter().take(TOP).map(|h| holding_text(h)).collect();
    lines.push(format!("• 最好：{}", best.join("、")));
    if held.len() > TOP {
        let worst: Vec<String> = held
            .iter()
            .rev()
            .take(TOP.min(held.len() - TOP))
            .map(|h| holding_text(h))
            .collect();
        lines.push(format!("• 最差：{}", worst.join("、")));
    }
    lines
}

#[cfg(test)]
mod tests;
