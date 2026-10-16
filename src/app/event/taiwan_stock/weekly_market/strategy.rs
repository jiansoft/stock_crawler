//! 每週市場摘要的策略一段：有設定策略換股通知的條件（見 [`super::super::strategy_alert`]）時，列出
//! 這一期換股日、本期至今的報酬與 0050 對照，以及表現最好、最差的幾檔持股。

use super::TOP;
use crate::app::{
    event::taiwan_stock::strategy_alert::{NAME_ENV, QUERY_ENV},
    ports::{self, StrategyHolding, StrategyRebalance},
};

/// 策略一段的資料：通知標題用的名稱與最後一次換股。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StrategySection {
    pub(super) name: String,
    pub(super) rebalance: StrategyRebalance,
}

/// 依 `.env` 的策略條件跑一次回測；沒有設定、gateway 未註冊、回測失敗或最後一期已滿時回 `None`
/// （失敗只記警告）。
pub(super) async fn strategy_section() -> Option<StrategySection> {
    let query = std::env::var(QUERY_ENV).unwrap_or_default();
    if query.trim().is_empty() {
        return None;
    }
    let gateway = match ports::strategy_gateway() {
        Ok(gateway) => gateway,
        Err(why) => {
            tracing::warn!("每週市場摘要：策略 gateway 無法使用: {why:#}");
            return None;
        }
    };
    match gateway.latest_rebalance(query.trim()).await {
        Ok(rebalance) => rebalance.map(|rebalance| StrategySection {
            name: std::env::var(NAME_ENV)
                .unwrap_or_default()
                .trim()
                .to_owned(),
            rebalance,
        }),
        Err(why) => {
            tracing::warn!("每週市場摘要：策略回測失敗: {why:#}");
            None
        }
    }
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

/// 策略一段的各行（開頭為空行）。
pub(super) fn lines(section: &StrategySection) -> Vec<String> {
    let rebalance = &section.rebalance;
    let title = if section.name.is_empty() {
        "策略持股".to_owned()
    } else {
        format!("策略持股（{}）", section.name)
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
