use anyhow::anyhow;
use async_trait::async_trait;

use super::*;
use crate::app::ports::{StrategyRebalance, StrategySettlement};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn window(median: Option<f64>, etf: Option<f64>, beat: Option<f64>) -> EdgeWindow {
    EdgeWindow {
        from: date(2024, 10, 28),
        to: date(2026, 10, 30),
        offsets: 20,
        median_return: median,
        etf_median_return: etf,
        beat_rate: beat,
    }
}

fn record(name: &str, recent: EdgeWindow, full: Option<EdgeWindow>) -> EdgeRecord {
    EdgeRecord {
        name: name.to_owned(),
        checked: date(2026, 11, 2),
        recent,
        full,
    }
}

#[test]
fn verdict_requires_median_above_etf_and_half_of_offsets() {
    assert_eq!(
        verdict(&window(Some(31.77), Some(30.36), Some(0.65))),
        Verdict::Holds
    );
    // 勝率剛好一半不算轉弱。
    assert_eq!(
        verdict(&window(Some(31.0), Some(30.0), Some(0.5))),
        Verdict::Holds
    );
    assert_eq!(
        verdict(&window(Some(31.0), Some(30.0), Some(0.45))),
        Verdict::Weakened(vec!["勝 0050 的換股日偏移只有 45%（門檻 50%）".to_owned()])
    );
    assert_eq!(
        verdict(&window(Some(28.5), Some(30.36), Some(0.3))),
        Verdict::Weakened(vec![
            "扣成本年化中位數 28.50% 低於 0050 的 30.36%".to_owned(),
            "勝 0050 的換股日偏移只有 30%（門檻 50%）".to_owned(),
        ])
    );
    assert_eq!(
        verdict(&window(None, Some(30.0), Some(0.5))),
        Verdict::Unknown
    );
    assert_eq!(
        verdict(&window(Some(30.0), None, Some(0.5))),
        Verdict::Unknown
    );
    assert_eq!(
        verdict(&window(Some(30.0), Some(29.0), None)),
        Verdict::Unknown
    );
}

#[test]
fn edge_query_overrides_dates_and_execution() {
    assert_eq!(
        edge_query(
            "preset=x&from=2019-01-02&to=2026-01-01&hold_days=20&execution_price=close&slippage_bps=5&rebalance_offset=3&limit=10&",
            date(2024, 11, 3)
        ),
        "preset=x&hold_days=20&limit=10&from=2024-11-03&execution_price=next_open&slippage_bps=20"
    );
    assert_eq!(
        edge_query("", date(2019, 1, 2)),
        "from=2019-01-02&execution_price=next_open&slippage_bps=20"
    );
}

#[test]
fn first_trading_day_skips_weekends_and_holidays() {
    let none = HashSet::new();
    // 2026-11-01 是週日，11-02（週一）是第一個交易日。
    assert!(!is_first_trading_day(date(2026, 11, 1), &none));
    assert!(is_first_trading_day(date(2026, 11, 2), &none));
    assert!(!is_first_trading_day(date(2026, 11, 3), &none));
    // 2026-10-01（週四）。
    assert!(is_first_trading_day(date(2026, 10, 1), &none));
    assert!(!is_first_trading_day(date(2026, 10, 2), &none));

    // 元旦休市：2027-01-01 週五休市、01-04 週一才是第一個交易日。
    let holidays: HashSet<NaiveDate> = [date(2027, 1, 1)].into_iter().collect();
    assert!(!is_first_trading_day(date(2027, 1, 1), &holidays));
    assert!(is_first_trading_day(date(2027, 1, 4), &holidays));
    assert!(!is_first_trading_day(date(2027, 1, 5), &holidays));
}

#[test]
fn texts_show_both_windows_and_disclaimer() {
    let recent = window(Some(28.5), Some(30.36), Some(0.3));
    let mut full = window(Some(31.77), Some(30.36), Some(0.65));
    full.from = date(2019, 1, 2);
    assert_eq!(
        window_text("近 24 個月", &recent),
        "近 24 個月（2024-10-28～2026-10-30，20 個偏移）：策略 28.50%／0050 30.36%，勝 0050 偏移 30%"
    );
    assert_eq!(
        window_text("近 24 個月", &window(None, None, None)),
        "近 24 個月（2024-10-28～2026-10-30，20 個偏移）：策略 —／0050 —，勝 0050 偏移 —"
    );

    let weak = record("最佳組合", recent.clone(), Some(full.clone()));
    let alerts = alerts(&[
        weak.clone(),
        record("穩健", window(Some(35.0), Some(30.0), Some(0.8)), None),
    ]);
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].0, "策略優勢轉弱：最佳組合");
    assert_eq!(
        alerts[0].1,
        [
            "最佳組合：扣成本年化中位數 28.50% 低於 0050 的 30.36%；勝 0050 的換股日偏移只有 30%（門檻 50%）",
            "近 24 個月（2024-10-28～2026-10-30，20 個偏移）：策略 28.50%／0050 30.36%，勝 0050 偏移 30%",
            "2019 起（2019-01-02～2026-10-30，20 個偏移）：策略 31.77%／0050 30.36%，勝 0050 偏移 65%",
            "成交假設：次日開盤＋20 bps，換股日偏移的中位數",
            "依 10-25 決議建議停用此策略的換股通知（strategy_preset.alert）；不會自動停用。",
            "僅供參考，不構成投資建議，亦不會自動下單。",
        ]
        .join("\n")
    );

    assert!(weekly_lines(&[]).is_empty());
    let unnamed = record("", window(Some(35.0), Some(30.0), Some(0.8)), None);
    assert_eq!(
        weekly_lines(&[weak, unnamed]),
        vec![
            String::new(),
            "策略優勢（2026-11-02 檢查，次日開盤＋20 bps，換股日偏移中位數）".to_owned(),
            "• ⚠️ 最佳組合 近 24 個月（2024-10-28～2026-10-30，20 個偏移）：策略 28.50%／0050 30.36%，勝 0050 偏移 30%".to_owned(),
            "• 最佳組合 2019 起（2019-01-02～2026-10-30，20 個偏移）：策略 31.77%／0050 30.36%，勝 0050 偏移 65%".to_owned(),
            "• 策略 近 24 個月（2024-10-28～2026-10-30，20 個偏移）：策略 35.00%／0050 30.00%，勝 0050 偏移 80%".to_owned(),
        ]
    );
}

#[test]
fn records_round_trip_through_json() {
    let records = vec![record(
        "最佳組合",
        window(Some(31.77), Some(30.36), Some(0.65)),
        None,
    )];
    let json = serde_json::to_string(&records).unwrap();
    let parsed: Vec<EdgeRecord> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, records);
    log_records(&[
        record("a", window(Some(31.0), Some(30.0), Some(0.6)), None),
        record(
            "b",
            window(Some(29.0), Some(30.0), Some(0.6)),
            Some(window(None, None, None)),
        ),
        record("c", window(None, None, None), None),
    ]);
}

/// 依起日回傳不同分布的假 gateway；條件含 `fail` 的近期一段失敗，含 `nofull` 的全期一段失敗。
struct FakeGateway;

#[async_trait]
impl StrategyGateway for FakeGateway {
    async fn latest_rebalance(&self, _: &str) -> anyhow::Result<Option<StrategyRebalance>> {
        unreachable!("策略優勢檢查不取換股")
    }

    async fn settlement(
        &self,
        _: &str,
        _: NaiveDate,
    ) -> anyhow::Result<Option<StrategySettlement>> {
        unreachable!("策略優勢檢查不做結算")
    }

    async fn offset_edge(&self, query: &str) -> anyhow::Result<StrategyOffsetEdge> {
        assert!(
            query.contains("execution_price=next_open&slippage_bps=20"),
            "{query}"
        );
        let full = query.contains("from=2019-01-02");
        if query.contains("fail") && !full {
            return Err(anyhow!("回測失敗"));
        }
        if query.contains("nofull") && full {
            return Err(anyhow!("全期失敗"));
        }
        Ok(StrategyOffsetEdge {
            from: if full {
                date(2019, 1, 2)
            } else {
                date(2024, 11, 4)
            },
            to: date(2026, 10, 30),
            offsets: 20,
            median_return: Some(if full { 31.77 } else { 28.0 }),
            etf_median_return: Some(30.36),
            beat_rate: Some(0.4),
        })
    }
}

#[tokio::test]
async fn check_runs_both_windows_and_collects_failures() {
    let strategies = vec![
        AlertStrategy {
            name: "最佳組合".to_owned(),
            query: "preset=best&from=2019-01-02".to_owned(),
        },
        AlertStrategy {
            name: "壞掉".to_owned(),
            query: "preset=fail".to_owned(),
        },
        AlertStrategy {
            name: "只有近期".to_owned(),
            query: "preset=nofull".to_owned(),
        },
    ];
    let (records, failures) = check(&FakeGateway, &strategies, date(2026, 11, 2)).await;
    assert_eq!(failures, vec!["壞掉：回測失敗".to_owned()]);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].name, "最佳組合");
    assert_eq!(records[0].checked, date(2026, 11, 2));
    assert_eq!(records[0].recent.median_return, Some(28.0));
    assert_eq!(
        records[0].full.as_ref().map(|full| full.median_return),
        Some(Some(31.77))
    );
    assert_eq!(records[1].name, "只有近期");
    assert_eq!(records[1].full, None);
    assert_eq!(alerts(&records).len(), 2);
}

/// 預設的 `offset_edge` 回錯誤（不支援偏移掃描的 gateway）。
#[tokio::test]
async fn default_offset_edge_is_unsupported() {
    struct Minimal;

    #[async_trait]
    impl StrategyGateway for Minimal {
        async fn latest_rebalance(&self, _: &str) -> anyhow::Result<Option<StrategyRebalance>> {
            Ok(None)
        }

        async fn settlement(
            &self,
            _: &str,
            _: NaiveDate,
        ) -> anyhow::Result<Option<StrategySettlement>> {
            Ok(None)
        }
    }

    assert!(Minimal.latest_rebalance("").await.unwrap().is_none());
    assert!(
        Minimal
            .settlement("", date(2026, 1, 1))
            .await
            .unwrap()
            .is_none()
    );
    let error = Minimal.offset_edge("from=2019-01-02").await.unwrap_err();
    assert!(error.to_string().contains("does not support"), "{error}");
}
