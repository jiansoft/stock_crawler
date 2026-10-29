//! 持股配置偏離提醒一段：無目標不輸出、偏離都在容忍帶內不輸出、超出容忍帶列前 N 項，
//! 以及資料來源失敗的降級與整則訊息的組合。

mod etf_look_through;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{NaiveDate, Utc};

use super::super::allocation::{
    AllocationDrift, AllocationSection, Direction, allocation_sections, lines, section_from_result,
};
use super::fakes::name;
use super::*;
use crate::domain::portfolio::{
    rebalance::{
        CurrentPosition, DatedEtfComponent, EtfLookThroughComponent, PortfolioPositionQuery,
        RebalanceParams, calculate_rebalance,
    },
    target::{PortfolioTargetRecord, PortfolioTargetRepository, TargetDimension},
};

struct FakeTargets {
    records: Vec<PortfolioTargetRecord>,
    fail: bool,
}

#[async_trait]
impl PortfolioTargetRepository for FakeTargets {
    async fn list_targets(
        &self,
        _member_id: &str,
        _dimension: Option<TargetDimension>,
    ) -> Result<Vec<PortfolioTargetRecord>> {
        if self.fail {
            return Err(anyhow!("目標讀取失敗"));
        }
        Ok(self.records.clone())
    }

    async fn has_configuration(&self, _: &str, _: TargetDimension) -> Result<bool> {
        Ok(!self.records.is_empty())
    }

    async fn save_targets(&self, _: &str, _: TargetDimension, _: &[(String, f64)]) -> Result<()> {
        Ok(())
    }
}

struct FakePositions {
    positions: Vec<CurrentPosition>,
    fail: bool,
}

#[async_trait]
impl PortfolioPositionQuery for FakePositions {
    async fn fetch_positions(&self, _: i64, _: &[String]) -> Result<Vec<CurrentPosition>> {
        if self.fail {
            return Err(anyhow!("持股讀取失敗"));
        }
        Ok(self.positions.clone())
    }
}

fn target(dimension: TargetDimension, key: &str, percent: f64) -> PortfolioTargetRecord {
    PortfolioTargetRecord {
        member_id: "0".to_owned(),
        dimension,
        key: key.to_owned(),
        target_percent: percent,
        updated_time: Utc::now(),
    }
}

fn position(symbol: &str, industry: &str, shares: i64, price: f64) -> CurrentPosition {
    CurrentPosition {
        symbol: symbol.to_owned(),
        name: name(symbol),
        shares,
        current_price: price,
        industry: industry.to_owned(),
        is_etf: false,
    }
}

/// 兩檔持股各 50%（2330 與 2317，皆 100 元 × 1000 股）。
fn even_positions() -> FakePositions {
    FakePositions {
        positions: vec![
            position("2330", "半導體業", 1000, 100.0),
            position("2317", "其他電子業", 1000, 100.0),
        ],
        fail: false,
    }
}

#[tokio::test]
async fn no_targets_is_silent() {
    let targets = FakeTargets {
        records: Vec::new(),
        fail: false,
    };
    assert!(
        allocation_sections(&targets, &even_positions(), today())
            .await
            .is_empty()
    );
    assert!(lines(&[]).is_empty());
}

#[tokio::test]
async fn deviations_inside_tolerance_are_silent() {
    let targets = FakeTargets {
        records: vec![
            target(TargetDimension::Stock, "2330", 51.0),
            target(TargetDimension::Stock, "2317", 49.0),
        ],
        fail: false,
    };
    assert!(
        allocation_sections(&targets, &even_positions(), today())
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn deviations_beyond_tolerance_are_listed_largest_first() {
    let targets = FakeTargets {
        records: vec![
            target(TargetDimension::Stock, "2330", 30.0),
            target(TargetDimension::Stock, "2317", 40.0),
            target(TargetDimension::Industry, "半導體業", 50.0),
            target(TargetDimension::Industry, "其他電子業", 50.0),
        ],
        fail: false,
    };
    let sections = allocation_sections(&targets, &even_positions(), today()).await;
    // 個股：2330 50→30（+20 減碼）、2317 50→40（+10 減碼）；產業在容忍帶內不列。
    assert_eq!(sections.len(), 1, "{sections:?}");
    assert_eq!(sections[0].dimension, TargetDimension::Stock);
    let labels: Vec<&str> = sections[0]
        .drifts
        .iter()
        .map(|drift| drift.label.as_str())
        .collect();
    assert_eq!(labels, ["2330 台積電", "2317 鴻海"]);
    assert!(
        sections[0]
            .drifts
            .iter()
            .all(|drift| drift.direction == Direction::Reduce)
    );
    let text = lines(&sections).join("\n");
    assert!(text.contains("持股配置偏離提醒"), "{text}");
    assert!(text.contains("2330 台積電 +20.0%（可考慮減碼）"), "{text}");
    assert!(text.contains("不會自動下單"), "{text}");
}

#[tokio::test]
async fn industry_dimension_and_underweight_direction() {
    let targets = FakeTargets {
        records: vec![target(TargetDimension::Industry, "半導體業", 80.0)],
        fail: false,
    };
    let sections = allocation_sections(&targets, &even_positions(), today()).await;
    assert_eq!(sections.len(), 1, "{sections:?}");
    assert_eq!(sections[0].dimension, TargetDimension::Industry);
    // 半導體業 50%→80% 低配 30 點；其他電子業沒有目標（0%）超配 50 點，排在前面。
    let drifts = &sections[0].drifts;
    assert_eq!(drifts[0].label, "其他電子業");
    assert_eq!(drifts[0].direction, Direction::Reduce);
    assert_eq!(drifts[1].label, "半導體業");
    assert_eq!(drifts[1].direction, Direction::Add);
    assert!(lines(&sections).join("\n").contains("• 產業："));
}

#[test]
fn only_top_five_drifts_are_kept() {
    let positions: Vec<CurrentPosition> = (1..=8)
        .map(|n| position(&format!("{n}"), "X", 1000, 100.0))
        .collect();
    let targets: Vec<(String, f64)> = (1..=8).map(|n| (format!("{n}"), 0.0)).collect();
    let result = calculate_rebalance(
        TargetDimension::Stock,
        &targets,
        &positions,
        &[],
        &RebalanceParams::default(),
    )
    .expect("試算成功");
    let section = section_from_result(&result).expect("全部偏離 12.5%");
    assert_eq!(section.drifts.len(), 5);
}

/// 沒有 ETF 持股時，ETF 穿透維度等同直接持有：台積電 100%、目標 10% → 超配 90 個百分點。
#[test]
fn etf_look_through_without_etfs_uses_direct_holdings() {
    let result = calculate_rebalance(
        TargetDimension::EtfLookThrough,
        &[("2330".to_owned(), 10.0)],
        &[position("2330", "半導體業", 1000, 100.0)],
        &[],
        &RebalanceParams::default(),
    )
    .expect("試算成功");
    let section = section_from_result(&result).expect("超出容忍帶");
    assert_eq!(section.dimension, TargetDimension::EtfLookThrough);
    assert_eq!(section.drifts[0].label, "2330 台積電");
    assert_eq!(section.drifts[0].deviation_percent, 90.0);
}

#[tokio::test]
async fn source_failures_and_unpriced_positions_degrade_silently() {
    let ok_targets = || FakeTargets {
        records: vec![target(TargetDimension::Stock, "2330", 30.0)],
        fail: false,
    };
    let failing_targets = FakeTargets {
        records: Vec::new(),
        fail: true,
    };
    assert!(
        allocation_sections(&failing_targets, &even_positions(), today())
            .await
            .is_empty()
    );
    let failing_positions = FakePositions {
        positions: Vec::new(),
        fail: true,
    };
    assert!(
        allocation_sections(&ok_targets(), &failing_positions, today())
            .await
            .is_empty()
    );
    // 缺收盤價（價格 0）時試算被拒絕，不能憑空算出偏離。
    let unpriced = FakePositions {
        positions: vec![position("2330", "半導體業", 1000, 0.0)],
        fail: false,
    };
    assert!(
        allocation_sections(&ok_targets(), &unpriced, today())
            .await
            .is_empty()
    );
}

#[test]
fn message_includes_allocation_only_when_drifting() {
    let section = AllocationSection {
        dimension: TargetDimension::Stock,
        drifts: vec![AllocationDrift {
            label: "2330 台積電".to_owned(),
            deviation_percent: 12.34,
            direction: Direction::Reduce,
        }],
        uncovered_etfs: Vec::new(),
    };
    let sections = [section];
    let message = build_message(
        None,
        None,
        None,
        &[],
        HoldingSection {
            allocation: &sections,
            ..Default::default()
        },
        None,
        name,
    )
    .expect("有偏離提醒");
    assert!(
        message.contains(&esc("📊 持股配置偏離提醒（容忍帶 ±2.0%）")),
        "{message}"
    );
    assert!(
        message.contains(&esc("• 個股：2330 台積電 +12.3%（可考慮減碼）")),
        "{message}"
    );
    assert!(build_message(None, None, None, &[], HoldingSection::default(), None, name).is_none());
    let empty = [AllocationSection {
        dimension: TargetDimension::Stock,
        drifts: Vec::new(),
        uncovered_etfs: Vec::new(),
    }];
    assert!(
        build_message(
            None,
            None,
            None,
            &[],
            HoldingSection {
                allocation: &empty,
                ..Default::default()
            },
            None,
            name,
        )
        .is_none()
    );
}

/// 週報的「今天」（ETF 成分資料是否過舊以此判斷）。
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 24).expect("日期應合法")
}
