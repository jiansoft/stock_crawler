//! 持股配置偏離提醒的 ETF 穿透：成分股曝險、過期成分股與失敗降級。

use super::*;

/// 持股與 ETF 成分股都由測試指定的假查詢。
struct FakeEtfPositions {
    positions: Vec<CurrentPosition>,
    components: Vec<DatedEtfComponent>,
    fail_components: bool,
}

#[async_trait]
impl PortfolioPositionQuery for FakeEtfPositions {
    async fn fetch_positions(&self, _: i64, _: &[String]) -> Result<Vec<CurrentPosition>> {
        Ok(self.positions.clone())
    }

    async fn fetch_etf_components(&self, etf_symbols: &[String]) -> Result<Vec<DatedEtfComponent>> {
        if self.fail_components {
            return Err(anyhow!("成分股讀取失敗"));
        }
        assert_eq!(etf_symbols, ["0050".to_owned()], "只查持有的 ETF");
        Ok(self.components.clone())
    }
}

fn component(stock: &str, weight: f64, as_of: NaiveDate) -> DatedEtfComponent {
    DatedEtfComponent {
        component: EtfLookThroughComponent {
            stock_symbol: stock.to_owned(),
            name: name(stock),
            weight_percent: weight,
            etf_symbol: "0050".to_owned(),
            industry: String::new(),
        },
        as_of,
    }
}

/// 0050 占 50%（成分台積電 50%、鴻海 10%）、直接持有台積電 20%、鴻海 30%。
fn etf_positions(as_of: NaiveDate, fail_components: bool) -> FakeEtfPositions {
    FakeEtfPositions {
        positions: vec![
            CurrentPosition {
                is_etf: true,
                ..position("0050", "", 1000, 50.0)
            },
            position("2330", "半導體業", 1000, 20.0),
            position("2317", "其他電子業", 1000, 30.0),
        ],
        components: vec![
            component("2330", 50.0, as_of),
            component("2317", 10.0, as_of),
        ],
        fail_components,
    }
}

fn etf_targets() -> FakeTargets {
    FakeTargets {
        records: vec![target(TargetDimension::EtfLookThrough, "2330", 30.0)],
        fail: false,
    }
}

/// 台積電穿透曝險 20%＋50%×50%＝45%，目標 30% → 超配 15 個百分點；沒設目標的鴻海不列。
#[tokio::test]
async fn etf_look_through_reports_exposure_including_etf_components() {
    let positions = etf_positions(today(), false);
    let sections = allocation_sections(&etf_targets(), &positions, today()).await;
    assert_eq!(
        sections,
        vec![AllocationSection {
            dimension: TargetDimension::EtfLookThrough,
            drifts: vec![AllocationDrift {
                label: "2330 台積電".to_owned(),
                deviation_percent: 15.0,
                direction: Direction::Reduce,
            }],
            uncovered_etfs: Vec::new(),
        }]
    );

    // 與再平衡頁同參數的試算結果一致。
    let components: Vec<_> = positions
        .components
        .iter()
        .map(|dated| dated.component.clone())
        .collect();
    let page = calculate_rebalance(
        TargetDimension::EtfLookThrough,
        &[("2330".to_owned(), 30.0)],
        &positions.positions,
        &components,
        &RebalanceParams {
            tolerance_percent: 2.0,
            ..RebalanceParams::default()
        },
    )
    .expect("試算應成功");
    let tsmc = page
        .etf_items
        .iter()
        .find(|item| item.stock_symbol == "2330")
        .expect("應有台積電");
    assert_eq!(tsmc.current_weight, 45.0);
    assert_eq!(tsmc.deviation_percent, -15.0);

    let lines = lines(&sections);
    assert!(
        lines.contains(&"• ETF 穿透：2330 台積電 +15.0%（可考慮減碼）".to_owned()),
        "{lines:?}"
    );
}

/// 成分資料超過 45 天：0050 不計入，台積電只剩直接持有 20%（低配 10%），並註明未計入。
#[tokio::test]
async fn etf_look_through_skips_stale_components_and_notes_them() {
    let stale = today() - chrono::Duration::days(46);
    let sections = allocation_sections(&etf_targets(), &etf_positions(stale, false), today()).await;
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].uncovered_etfs, vec!["0050".to_owned()]);
    assert_eq!(sections[0].drifts[0].deviation_percent, -10.0);
    assert_eq!(sections[0].drifts[0].direction, Direction::Add);
    let lines = lines(&sections);
    assert!(
        lines.contains(
            &"• ETF 穿透：2330 台積電 -10.0%（可考慮加碼）（0050 成分資料過舊或查無，未計入）"
                .to_owned()
        ),
        "{lines:?}"
    );

    // 第 45 天仍算新鮮。
    let edge = today() - chrono::Duration::days(45);
    let sections = allocation_sections(&etf_targets(), &etf_positions(edge, false), today()).await;
    assert!(sections[0].uncovered_etfs.is_empty());
}

/// 成分股讀取失敗只略過 ETF 穿透，個股維度照常輸出；在容忍帶內則不輸出。
#[tokio::test]
async fn etf_look_through_degrades_on_failure_and_stays_silent_within_tolerance() {
    let targets = FakeTargets {
        records: vec![
            target(TargetDimension::EtfLookThrough, "2330", 30.0),
            target(TargetDimension::Stock, "2330", 50.0),
        ],
        fail: false,
    };
    let sections = allocation_sections(&targets, &etf_positions(today(), true), today()).await;
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].dimension, TargetDimension::Stock);

    let within = FakeTargets {
        records: vec![target(TargetDimension::EtfLookThrough, "2330", 44.0)],
        fail: false,
    };
    assert!(
        allocation_sections(&within, &etf_positions(today(), false), today())
            .await
            .is_empty()
    );
}

/// 預設實作（不支援穿透的查詢）回空成分清單。
#[tokio::test]
async fn default_component_query_is_empty() {
    let components = even_positions()
        .fetch_etf_components(&["0050".to_owned()])
        .await
        .expect("預設實作不應失敗");
    assert!(components.is_empty());
}
