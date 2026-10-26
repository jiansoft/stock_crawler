//! 每週市場摘要的持股配置偏離提醒：預設組合（`member_id = 0`）有設定目標權重時，
//! 以 [`calculate_rebalance`] 試算偏離，只列出超出容忍帶的項目（最多 [`MAX_ITEMS`] 項）。
//!
//! 靜默原則：沒有設定目標、或所有偏離都在容忍帶內時不輸出任何內容，避免增加通知雜訊。
//! 只涵蓋個股與產業兩個維度；ETF 穿透需成分股權重，改在網頁與 MCP 試算。

use crate::domain::portfolio::{
    rebalance::{
        PortfolioPositionQuery, PortfolioRebalanceResult, RebalanceParams, calculate_rebalance,
    },
    target::{PortfolioTargetRepository, TargetDimension},
};

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
        TargetDimension::EtfLookThrough => return None,
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
    })
}

/// 載入預設組合的目標與持股，試算個股與產業維度的偏離；任何一步失敗只記警告並略過該維度。
pub(super) async fn allocation_sections(
    targets: &impl PortfolioTargetRepository,
    positions: &impl PortfolioPositionQuery,
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
    for dimension in [TargetDimension::Stock, TargetDimension::Industry] {
        let targets: Vec<(String, f64)> = records
            .iter()
            .filter(|record| record.dimension == dimension)
            .map(|record| (record.key.clone(), record.target_percent))
            .collect();
        if targets.is_empty() {
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
        lines.push(format!(
            "• {}：{}",
            dimension_label(section.dimension),
            items.join("、")
        ));
    }
    lines.push("＊僅供參考，不構成投資建議，亦不會自動下單".to_owned());
    lines
}
