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
        if query == "none" {
            return Ok(None);
        }
        if query == "bad" {
            bail!("backtest down");
        }
        Ok(Some(
            crate::app::event::taiwan_stock::strategy_alert::tests::rebalance(),
        ))
    }

    async fn settlement(&self, name: &str, date: NaiveDate) -> Result<Option<StrategySettlement>> {
        assert_eq!(date.to_string(), "2026-10-18");
        assert!(!name.is_empty(), "未命名策略也應使用換股紀錄的鍵");
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

/// 回傳固定策略或模擬讀取失敗的倉儲，其他操作不參與週報。
struct Repository {
    fail: bool,
    presets: Vec<StrategyPreset>,
}

#[async_trait]
impl crate::domain::strategy::StrategyPresetRepository for Repository {
    async fn list(&self) -> Result<Vec<StrategyPreset>> {
        if self.fail {
            bail!("repository unavailable");
        }
        Ok(self.presets.clone())
    }
    async fn find(&self, _: &str) -> Result<Option<StrategyPreset>> {
        unreachable!()
    }
    async fn save(&self, _: &StrategyPreset) -> Result<()> {
        unreachable!()
    }
    async fn delete(&self, _: &str) -> Result<bool> {
        unreachable!()
    }
}

/// 倉儲與 gateway 失敗時保留策略優勢；沒有資料時回 None，環境策略仍可單獨執行。
#[tokio::test]
async fn section_sources_degrade_independently() {
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .finish(),
    );
    let today = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
    let empty = Repository {
        fail: false,
        presets: Vec::new(),
    };
    assert!(
        load_section(
            &empty,
            Err(anyhow::anyhow!("missing gateway")),
            None,
            None,
            Vec::new(),
            today
        )
        .await
        .is_none()
    );
    let failed = Repository {
        fail: true,
        presets: Vec::new(),
    };
    let section = load_section(
        &failed,
        Ok(std::sync::Arc::new(Gateway)),
        Some("good".to_owned()),
        Some("環境策略".to_owned()),
        Vec::new(),
        today,
    )
    .await
    .unwrap();
    assert_eq!(section.positions[0].name, "環境策略");
    assert!(
        load_section(
            &empty,
            Ok(std::sync::Arc::new(Gateway)),
            Some("none".to_owned()),
            None,
            Vec::new(),
            today
        )
        .await
        .is_none()
    );
    let window = strategy_edge::EdgeWindow {
        from: today,
        to: today,
        offsets: 20,
        median_return: None,
        etf_median_return: None,
        beat_rate: None,
    };
    let edges = vec![EdgeRecord {
        name: "最佳組合".to_owned(),
        checked: today,
        recent: window,
        full: None,
    }];
    let section = load_section(
        &empty,
        Err(anyhow::anyhow!("missing gateway")),
        Some("good".to_owned()),
        None,
        edges.clone(),
        today,
    )
    .await
    .unwrap();
    assert!(section.positions.is_empty());
    assert_eq!(lines(&section), strategy_edge::weekly_lines(&edges));
}

/// 整合驗證真正的倉儲及 Redis 膠水；只在隔離服務環境執行。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要隔離 PostgreSQL 與 Redis"
)]
async fn section_entry_reads_real_sources() {
    let _ = strategy_section().await;
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

/// 未命名環境策略仍能讀取以 STRATEGY_ALERT_QUERY 為鍵的實盤紀錄。
#[tokio::test]
async fn unnamed_env_strategy_uses_record_key() {
    let strategies = alert_strategies(Vec::new(), Some("good".to_owned()), None);
    let positions = load_positions(
        &Gateway,
        &strategies,
        NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
    )
    .await;
    assert_eq!(positions.len(), 1);
    assert!(positions[0].name.is_empty());
    assert!(positions[0].settlement.is_some());
}
