//! 每週市場摘要的持股配置偏離提醒：預設組合（`member_id = 0`）有設定目標權重時，
//! 以 [`calculate_rebalance`] 試算偏離，只列出超出容忍帶的項目（最多 [`MAX_ITEMS`] 項）。
//!
//! 靜默原則：沒有設定目標、或所有偏離都在容忍帶內時不輸出任何內容，避免增加通知雜訊。
//! 涵蓋個股、產業與 ETF 穿透三個維度。ETF 穿透以持有 ETF 的前十大成分（與再平衡頁同一份來源）
//! 試算，只列使用者設了目標的個股；成分資料超過 [`STALE_COMPONENT_DAYS`] 天的 ETF 不計入並註明。

use std::collections::HashSet;

use chrono::NaiveDate;

use crate::domain::portfolio::{
    rebalance::{
        CurrentPosition, EtfLookThroughComponent, PortfolioPositionQuery, PortfolioRebalanceResult,
        RebalanceParams, calculate_rebalance,
    },
    target::{PortfolioTargetRepository, TargetDimension},
};

/// ETF 成分資料超過這麼多天（日曆天）未更新就視為過舊，不計入穿透。
const STALE_COMPONENT_DAYS: i64 = 45;

/// 週報使用的容忍帶（與 Data API 預設一致）。
pub(super) const TOLERANCE_PERCENT: f64 = 2.0;

/// 每個維度最多列出的偏離項目數。
const MAX_ITEMS: usize = 5;

/// 週報提醒的預設組合。
const MEMBER_ID: i64 = 0;

/// 偏離方向：目前權重高於目標建議減碼，低於目標建議加碼。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Direction {
    Reduce,
    Add,
}

/// 一個超出容忍帶的項目。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AllocationDrift {
    pub(super) label: String,
    /// 目前權重減目標權重（百分點）；正值代表超配、負值代表低配。
    pub(super) deviation_percent: f64,
    pub(super) direction: Direction,
}

/// 單一維度的偏離提醒；`drifts` 已依偏離絕對值由大到小排序並截斷。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AllocationSection {
    pub(super) dimension: TargetDimension,
    pub(super) drifts: Vec<AllocationDrift>,
    /// ETF 穿透維度：成分資料過舊或查無成分、沒有計入穿透的 ETF。
    pub(super) uncovered_etfs: Vec<String>,
}

fn direction(deviation: f64) -> Direction {
    if deviation > 0.0 {
        Direction::Reduce
    } else {
        Direction::Add
    }
}

/// 由試算結果取出超出容忍帶的項目；沒有任何項目超出時回 `None`。
pub(super) fn section_from_result(result: &PortfolioRebalanceResult) -> Option<AllocationSection> {
    let tolerance = result.tolerance_percent;
    let rows: Vec<(String, f64)> = match result.dimension {
        TargetDimension::Stock => result
            .stock_items
            .iter()
            .map(|item| {
                (
                    format!("{} {}", item.symbol, item.name),
                    -item.deviation_percent,
                )
            })
            .collect(),
        TargetDimension::Industry => result
            .industry_items
            .iter()
            .map(|item| (item.industry.clone(), -item.deviation_percent))
            .collect(),
        TargetDimension::EtfLookThrough => result
            .etf_items
            .iter()
            .map(|item| {
                (
                    format!("{} {}", item.stock_symbol, item.name),
                    -item.deviation_percent,
                )
            })
            .collect(),
    };
    let mut drifts: Vec<AllocationDrift> = rows
        .into_iter()
        .filter(|(_, deviation)| deviation.abs() >= tolerance)
        .map(|(label, deviation_percent)| AllocationDrift {
            label: label.trim().to_owned(),
            deviation_percent,
            direction: direction(deviation_percent),
        })
        .collect();
    if drifts.is_empty() {
        return None;
    }
    drifts.sort_by(|a, b| {
        b.deviation_percent
            .abs()
            .total_cmp(&a.deviation_percent.abs())
            .then_with(|| a.label.cmp(&b.label))
    });
    drifts.truncate(MAX_ITEMS);
    Some(AllocationSection {
        dimension: result.dimension,
        drifts,
        uncovered_etfs: Vec::new(),
    })
}

/// 持有 ETF 的成分股（只保留資料日期在 `today` 前 [`STALE_COMPONENT_DAYS`] 天內的 ETF），
/// 以及沒有計入的 ETF（過舊或查無成分，依代號排序）。
fn fresh_components(
    held: &[CurrentPosition],
    components: Vec<crate::domain::portfolio::rebalance::DatedEtfComponent>,
    today: NaiveDate,
) -> (Vec<EtfLookThroughComponent>, Vec<String>) {
    let oldest = today - chrono::Duration::days(STALE_COMPONENT_DAYS);
    let mut fresh_etfs = HashSet::new();
    let fresh: Vec<EtfLookThroughComponent> = components
        .into_iter()
        .filter(|dated| dated.as_of >= oldest)
        .map(|dated| {
            fresh_etfs.insert(dated.component.etf_symbol.clone());
            dated.component
        })
        .collect();
    let mut uncovered: Vec<String> = held
        .iter()
        .filter(|position| position.is_etf && position.shares > 0)
        .filter(|position| !fresh_etfs.contains(&position.symbol))
        .map(|position| position.symbol.clone())
        .collect();
    uncovered.sort();
    (fresh, uncovered)
}

/// ETF 穿透維度的偏離：成分股查詢失敗時記警告並略過；只列設了目標的個股。
async fn etf_look_through_section(
    positions: &impl PortfolioPositionQuery,
    targets: &[(String, f64)],
    held: &[CurrentPosition],
    params: &RebalanceParams,
    today: NaiveDate,
) -> Option<AllocationSection> {
    let etf_symbols: Vec<String> = held
        .iter()
        .filter(|position| position.is_etf && position.shares > 0)
        .map(|position| position.symbol.clone())
        .collect();
    let components = match positions.fetch_etf_components(&etf_symbols).await {
        Ok(components) => components,
        Err(why) => {
            tracing::warn!("每週市場摘要：讀取 ETF 成分股失敗: {why:#}");
            return None;
        }
    };
    let (components, uncovered_etfs) = fresh_components(held, components, today);
    let mut result = match calculate_rebalance(
        TargetDimension::EtfLookThrough,
        targets,
        held,
        &components,
        params,
    ) {
        Ok(result) => result,
        Err(why) => {
            tracing::warn!("每週市場摘要：ETF 穿透維度再平衡試算失敗: {why:#}");
            return None;
        }
    };
    // 沒設目標的成分股（例如 0050 其他前十大）目標視為 0，會被列成「減碼」，但那只能透過賣 ETF 調整，
    // 週報只提醒使用者明確設了目標的個股。
    let keys: HashSet<&str> = targets.iter().map(|(key, _)| key.as_str()).collect();
    result
        .etf_items
        .retain(|item| keys.contains(item.stock_symbol.as_str()));
    section_from_result(&result).map(|section| AllocationSection {
        uncovered_etfs,
        ..section
    })
}

/// 載入預設組合的目標與持股，試算個股、產業與 ETF 穿透維度的偏離；任何一步失敗只記警告並略過該維度。
/// `today` 用來判斷 ETF 成分資料是否過舊。
pub(super) async fn allocation_sections(
    targets: &impl PortfolioTargetRepository,
    positions: &impl PortfolioPositionQuery,
    today: NaiveDate,
) -> Vec<AllocationSection> {
    let records = match targets.list_targets(&MEMBER_ID.to_string(), None).await {
        Ok(records) => records,
        Err(why) => {
            tracing::warn!("每週市場摘要：讀取目標配置失敗: {why:#}");
            return Vec::new();
        }
    };
    if records.is_empty() {
        return Vec::new();
    }
    let extra: Vec<String> = records
        .iter()
        .filter(|record| record.dimension == TargetDimension::Stock)
        .map(|record| record.key.clone())
        .collect();
    let held = match positions.fetch_positions(MEMBER_ID, &extra).await {
        Ok(held) => held,
        Err(why) => {
            tracing::warn!("每週市場摘要：讀取持股部位失敗: {why:#}");
            return Vec::new();
        }
    };
    let params = RebalanceParams {
        tolerance_percent: TOLERANCE_PERCENT,
        ..RebalanceParams::default()
    };
    let mut sections = Vec::new();
    for dimension in [
        TargetDimension::Stock,
        TargetDimension::Industry,
        TargetDimension::EtfLookThrough,
    ] {
        let targets: Vec<(String, f64)> = records
            .iter()
            .filter(|record| record.dimension == dimension)
            .map(|record| (record.key.clone(), record.target_percent))
            .collect();
        if targets.is_empty() {
            continue;
        }
        if dimension == TargetDimension::EtfLookThrough {
            sections
                .extend(etf_look_through_section(positions, &targets, &held, &params, today).await);
            continue;
        }
        match calculate_rebalance(dimension, &targets, &held, &[], &params) {
            Ok(result) => sections.extend(section_from_result(&result)),
            Err(why) => tracing::warn!(
                "每週市場摘要：{}維度再平衡試算失敗: {why:#}",
                dimension.as_str()
            ),
        }
    }
    sections
}

fn dimension_label(dimension: TargetDimension) -> &'static str {
    match dimension {
        TargetDimension::Stock => "個股",
        TargetDimension::Industry => "產業",
        TargetDimension::EtfLookThrough => "ETF 穿透",
    }
}

/// 偏離提醒一段的各行（開頭為空行）；沒有任何超出容忍帶的項目時回空陣列。
pub(super) fn lines(sections: &[AllocationSection]) -> Vec<String> {
    if sections.iter().all(|section| section.drifts.is_empty()) {
        return Vec::new();
    }
    let mut lines = vec![
        String::new(),
        format!("📊 持股配置偏離提醒（容忍帶 ±{TOLERANCE_PERCENT:.1}%）"),
    ];
    for section in sections.iter().filter(|section| !section.drifts.is_empty()) {
        let items: Vec<String> = section
            .drifts
            .iter()
            .map(|drift| {
                let advice = match drift.direction {
                    Direction::Reduce => "可考慮減碼",
                    Direction::Add => "可考慮加碼",
                };
                format!(
                    "{} {:+.1}%（{advice}）",
                    drift.label, drift.deviation_percent
                )
            })
            .collect();
        let note = if section.uncovered_etfs.is_empty() {
            String::new()
        } else {
            format!(
                "（{} 成分資料過舊或查無，未計入）",
                section.uncovered_etfs.join("、")
            )
        };
        lines.push(format!(
            "• {}：{}{note}",
            dimension_label(section.dimension),
            items.join("、")
        ));
    }
    lines.push("＊僅供參考，不構成投資建議，亦不會自動下單".to_owned());
    lines
}
