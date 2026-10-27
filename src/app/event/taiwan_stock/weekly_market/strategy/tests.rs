//! 多策略週報的資料載入、去重與個別失敗降級測試。

use super::*;
use crate::domain::strategy::StrategyPreset;
use anyhow::{Result, bail};
use async_trait::async_trait;
use chrono::NaiveDate;

/// 特定條件回測失敗，特定名稱結算失敗，其餘回固定持股與實盤累積。
struct Gateway;

#[async_trait]
impl StrategyGateway for Gateway {
    async fn latest_rebalance(&self, query: &str) -> Result<Option<StrategyRebalance>> {
        if query == "bad" {
            bail!("backtest down");
        }
        Ok(Some(
            crate::app::event::taiwan_stock::strategy_alert::tests::rebalance(),
        ))
    }

    async fn settlement(&self, name: &str, date: NaiveDate) -> Result<Option<StrategySettlement>> {
        assert_eq!(date.to_string(), "2026-10-18");
        if name == "無結算" {
            bail!("settlement down");
        }
        let day = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        Ok(Some(StrategySettlement {
            start: day,
            end: day,
            strategy_return: Some(3.0),
            etf_return: Some(1.0),
            portfolio_return: Some(2.0),
            etf_weight: 50,
            tracked_from: day,
            periods: 2,
            cumulative_return: Some(5.5),
            etf_cumulative_return: Some(1.25),
        }))
    }
}

/// 具名與環境變數的條件相同只跑一次，回測與結算失敗各自降級。
#[tokio::test]
async fn named_strategies_deduplicate_and_degrade_independently() {
    let preset = |name: &str, query: &str| StrategyPreset {
        name: name.to_owned(),
        query: query.to_owned(),
        description: String::new(),
        alert: true,
    };
    let strategies = alert_strategies(
        vec![
            preset("最佳組合", "good"),
            preset("失敗", "bad"),
            preset("無結算", "other"),
        ],
        Some("good".to_owned()),
        None,
    );
    assert_eq!(strategies.len(), 3);
    let positions = load_positions(
        &Gateway,
        &strategies,
        NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
    )
    .await;
    assert_eq!(positions.len(), 2);
    assert_eq!(positions[0].name, "最佳組合");
    assert!(positions[0].settlement.is_some());
    assert!(positions[1].settlement.is_none());
    let rendered = lines(&StrategySection {
        positions,
        edges: Vec::new(),
    })
    .join("\n");
    assert!(rendered.contains("策略持股（最佳組合）"));
    assert!(rendered.contains("累積 +5.50%，0050 +1.25%"));
    assert!(rendered.contains("策略持股（無結算）"));
    assert!(!rendered.contains("策略持股（失敗）"));
}

/// 沒有任何策略時不呼叫閘道，仍能獨立保留優勢段。
#[tokio::test]
async fn no_strategies_load_no_positions() {
    assert!(
        load_positions(
            &Gateway,
            &[],
            NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()
        )
        .await
        .is_empty()
    );
}
