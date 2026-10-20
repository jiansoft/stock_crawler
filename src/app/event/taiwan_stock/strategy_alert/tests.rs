//! 策略換股通知的測試：策略清單、換股紀錄、通知流程與訊息內容。

use std::sync::Mutex;

use anyhow::bail;
use async_trait::async_trait;

use super::message::build_message;
use super::*;
use crate::app::ports::{StrategyHolding, StrategySettlement};
use crate::domain::portfolio::health::{FundamentalSignals, HoldingFundamentals};

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("固定日期合法")
}

pub(in crate::app::event::taiwan_stock) fn holding(
    symbol: &str,
    name: Option<&str>,
    weight: f64,
) -> StrategyHolding {
    StrategyHolding {
        symbol: symbol.to_owned(),
        name: name.map(str::to_owned),
        weight,
        return_to_date: None,
    }
}

pub(in crate::app::event::taiwan_stock) fn rebalance() -> StrategyRebalance {
    StrategyRebalance {
        date: day(16),
        hold_days: 20,
        days_elapsed: 0,
        return_to_date: None,
        etf_return_to_date: None,
        etf_weight: 0,
        market_filtered: false,
        cash: 0.0,
        turnover: 0.25,
        added: vec![holding("2330", Some("台積電"), 50.0)],
        kept: vec![holding("2317", Some("鴻海"), 50.0)],
        exited: vec![holding("79979", None, 50.0)],
    }
}

/// 回傳固定持股基本面的假查詢；`fail` 時查詢失敗。
#[derive(Default)]
struct FakeFundamentals {
    fundamentals: Vec<HoldingFundamentals>,
    fail: bool,
}

impl FakeFundamentals {
    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }
}

#[async_trait]
impl HoldingFundamentalsQuery for FakeFundamentals {
    async fn fetch_holding_fundamentals(
        &self,
        _symbols: &[String],
    ) -> Result<Vec<HoldingFundamentals>> {
        if self.fail {
            bail!("db down");
        }
        Ok(self.fundamentals.clone())
    }
}

/// 記下寫入的換股紀錄。
#[derive(Default)]
struct FakeRecorder {
    saved: Mutex<Vec<StrategyRebalanceRecord>>,
}

#[async_trait]
impl StrategyRebalanceRepository for FakeRecorder {
    async fn save(&self, record: &StrategyRebalanceRecord) -> Result<bool> {
        self.saved.lock().expect("鎖未中毒").push(record.clone());
        Ok(true)
    }

    async fn list(&self, _preset_name: &str) -> Result<Vec<StrategyRebalanceRecord>> {
        Ok(Vec::new())
    }
}

/// 換股紀錄：新買進在前、續抱在後，賣出的不記；沒有名稱的策略以環境變數名稱記錄。
#[test]
fn rebalance_record_keeps_added_then_kept() {
    let strategy = AlertStrategy {
        name: "最佳組合".to_owned(),
        query: "from=2019-01-02".to_owned(),
    };
    let core = StrategyRebalance {
        etf_weight: 50,
        cash: 10.0,
        ..rebalance()
    };
    assert_eq!(
        rebalance_record(&strategy, &core),
        StrategyRebalanceRecord {
            preset_name: "最佳組合".to_owned(),
            date: day(16),
            query: "from=2019-01-02".to_owned(),
            hold_days: 20,
            etf_weight: 50,
            cash: 10.0,
            market_filtered: false,
            holdings: vec![
                RecordedHolding {
                    symbol: "2330".to_owned(),
                    name: Some("台積電".to_owned()),
                    weight: 50.0,
                },
                RecordedHolding {
                    symbol: "2317".to_owned(),
                    name: Some("鴻海".to_owned()),
                    weight: 50.0,
                },
            ],
        }
    );
    let unnamed = AlertStrategy {
        name: String::new(),
        ..strategy
    };
    assert_eq!(
        rebalance_record(&unnamed, &rebalance()).preset_name,
        UNNAMED_STRATEGY
    );
}

/// 回傳固定結果並記下收到的條件與結算的策略。
struct FakeGateway {
    result: Option<StrategyRebalance>,
    fail: bool,
    settlement: Option<StrategySettlement>,
    settlement_fail: bool,
    settled: Mutex<Vec<(String, NaiveDate)>>,
    queries: Mutex<Vec<String>>,
}

#[async_trait]
impl StrategyGateway for FakeGateway {
    async fn latest_rebalance(&self, query: &str) -> Result<Option<StrategyRebalance>> {
        self.queries
            .lock()
            .expect("鎖未中毒")
            .push(query.to_owned());
        if self.fail {
            bail!("回測條件錯誤");
        }
        Ok(self.result.clone())
    }

    async fn settlement(
        &self,
        preset_name: &str,
        date: NaiveDate,
    ) -> Result<Option<StrategySettlement>> {
        self.settled
            .lock()
            .expect("鎖未中毒")
            .push((preset_name.to_owned(), date));
        if self.settlement_fail {
            bail!("讀不到換股紀錄");
        }
        Ok(self.settlement.clone())
    }
}

fn gateway(result: Option<StrategyRebalance>) -> FakeGateway {
    FakeGateway {
        result,
        fail: false,
        settlement: None,
        settlement_fail: false,
        settled: Mutex::new(Vec::new()),
        queries: Mutex::new(Vec::new()),
    }
}

fn settlement(etf_weight: u8) -> StrategySettlement {
    StrategySettlement {
        start: NaiveDate::from_ymd_opt(2026, 9, 17).expect("固定日期合法"),
        end: day(16),
        strategy_return: Some(2.034),
        etf_return: Some(-4.21),
        portfolio_return: Some(-1.09),
        etf_weight,
        tracked_from: NaiveDate::from_ymd_opt(2026, 8, 19).expect("固定日期合法"),
        periods: 2,
        cumulative_return: Some(5.5),
        etf_cumulative_return: Some(1.25),
    }
}

/// 有上一期結算時附兩行：上一期組合／策略部位／0050，追蹤以來累積與超額；純策略只列策略與 0050，
/// 查不到的寫「查不到」、沒有超額。
#[test]
fn build_message_includes_settlement() {
    let message = build_message(&rebalance(), "", None, Some(&settlement(50)), &[]);
    let expected = [
        r"• 上一期 2026\-09\-17～2026\-10\-16 實盤（未扣成本）：組合 \-1\.09%（策略部位 \+2\.03%，0050 \-4\.21%）",
        r"• 追蹤 2026\-08\-19 起 2 期：累積 \+5\.50%，0050 \+1\.25%，超額 \+4\.25 個百分點",
        "",
    ]
    .join("\n");
    assert!(message.contains(&expected), "{message}");
    let pure = StrategySettlement {
        strategy_return: None,
        cumulative_return: None,
        ..settlement(0)
    };
    let message = build_message(&rebalance(), "", None, Some(&pure), &[]);
    assert!(
        message.contains("實盤（未扣成本）：策略 查不到，0050 \\-4\\.21%\n"),
        "{message}"
    );
    assert!(
        message.contains("累積 查不到，0050 \\+1\\.25%\n"),
        "{message}"
    );
    assert!(!message.contains("超額"), "{message}");
    assert!(!build_message(&rebalance(), "", None, None, &[]).contains("上一期"));
}

/// 新買進、續抱、賣出各一行，附換手與下次換股。
#[test]
fn build_message_lists_changes() {
    let message = build_message(&rebalance(), "動能", None, None, &[]);
    assert_eq!(
        message,
        [
            "策略換股（動能）︰2026\\-10\\-16",
            "• 持有 2 檔",
            "• 新買進（1 檔）：2330 台積電 50\\.0%",
            "• 續抱（1 檔）：2317 鴻海 50\\.0%",
            "• 賣出（1 檔）：79979 50\\.0%",
            "• 換手 25\\.0%",
            "• 下次換股：20 個交易日後",
            "",
            "＊依回測條件以今天收盤選出，實際下單在下一個交易日；過去表現不代表未來結果",
            "",
        ]
        .join("\n")
    );
}

/// 大盤濾網空手、沒有符合條件、有現金與 0050 核心部位的寫法。
#[test]
fn build_message_describes_cash_and_core() {
    let filtered = StrategyRebalance {
        market_filtered: true,
        added: Vec::new(),
        kept: Vec::new(),
        etf_weight: 50,
        ..rebalance()
    };
    let message = build_message(&filtered, "", None, None, &[]);
    assert!(
        message.starts_with("策略換股︰2026\\-10\\-16\n"),
        "{message}"
    );
    assert!(
        message.contains("大盤濾網沒過，策略部位全部轉為現金"),
        "{message}"
    );
    assert!(message.contains("0050 核心部位 50%"), "{message}");
    assert!(!message.contains("新買進"), "{message}");

    let empty = StrategyRebalance {
        added: Vec::new(),
        kept: Vec::new(),
        ..rebalance()
    };
    assert!(build_message(&empty, "", None, None, &[]).contains("沒有符合條件的股票"));

    let capped = StrategyRebalance {
        cash: 40.0,
        ..rebalance()
    };
    assert!(
        build_message(&capped, "", None, None, &[]).contains("策略部位現金 40\\.0%"),
        "{}",
        build_message(&capped, "", None, None, &[])
    );
}

/// 對照實際持股：已持有與尚未持有、策略賣出但仍持有、0050 核心部位；讀不到持股時不列。
#[test]
fn build_message_compares_with_actual_holdings() {
    let held: BTreeSet<String> = ["2317", "79979", "0050"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let message = build_message(&rebalance(), "", Some(&held), None, &[]);
    assert!(
        message.contains(
            "• 對照實際持股：策略 2 檔中已持有 1 檔；尚未持有：2330 台積電
"
        ),
        "{message}"
    );
    assert!(
        message.contains(
            "• 策略賣出但仍持有：79979
"
        ),
        "{message}"
    );
    assert!(!message.contains("0050 核心部位："), "{message}");

    let core = StrategyRebalance {
        etf_weight: 50,
        exited: Vec::new(),
        ..rebalance()
    };
    let message = build_message(&core, "", Some(&held), None, &[]);
    assert!(message.contains("• 0050 核心部位：已持有"), "{message}");
    assert!(!message.contains("仍持有"), "{message}");
    let all: BTreeSet<String> = ["2330", "2317"].into_iter().map(str::to_owned).collect();
    let message = build_message(&core, "", Some(&all), None, &[]);
    assert!(
        message.contains(
            "已持有 2 檔
"
        ),
        "{message}"
    );
    assert!(message.contains("• 0050 核心部位：尚未持有"), "{message}");

    let filtered = StrategyRebalance {
        market_filtered: true,
        added: Vec::new(),
        kept: Vec::new(),
        ..rebalance()
    };
    let message = build_message(&filtered, "", Some(&held), None, &[]);
    assert!(!message.contains("對照實際持股"), "{message}");
    assert!(message.contains("策略賣出但仍持有：79979"), "{message}");
    assert!(!build_message(&rebalance(), "", None, None, &[]).contains("對照實際持股"));
}

fn preset(name: &str, query: &str, alert: bool) -> StrategyPreset {
    StrategyPreset {
        name: name.to_owned(),
        query: query.to_owned(),
        description: String::new(),
        alert,
    }
}

/// 只取 `alert` 的具名策略；環境變數的條件空白或與具名策略相同時不重複加入。
#[test]
fn alert_strategies_merge_presets_and_env() {
    let presets = vec![
        preset("甲", " from=2019-01-02&limit=20 ", true),
        preset("乙", "from=2020-01-02", false),
    ];
    let strategies = alert_strategies(
        presets.clone(),
        Some("from=2021-01-04".to_owned()),
        Some(" 環境 ".to_owned()),
    );
    assert_eq!(
        strategies,
        [
            AlertStrategy {
                name: "甲".to_owned(),
                query: "from=2019-01-02&limit=20".to_owned()
            },
            AlertStrategy {
                name: "環境".to_owned(),
                query: "from=2021-01-04".to_owned()
            },
        ]
    );
    let same = alert_strategies(
        presets.clone(),
        Some("from=2019-01-02&limit=20".to_owned()),
        None,
    );
    assert_eq!(same.len(), 1);
    assert_eq!(
        alert_strategies(presets, Some("  ".to_owned()), None).len(),
        1
    );
    assert!(alert_strategies(Vec::new(), None, None).is_empty());
}

/// 逐一執行：今天不換股的不發、失敗的記下但不中斷其他策略，最後回傳錯誤；沒有策略不需要 gateway。
#[tokio::test]
async fn notify_continues_after_failures() {
    let strategy = |name: &str| AlertStrategy {
        name: name.to_owned(),
        query: format!("from=2019-01-02&name={name}"),
    };
    let recorder = FakeRecorder::default();
    let idle = gateway(None);
    notify(
        &idle,
        &recorder,
        &FakeFundamentals::default(),
        &[strategy("甲"), strategy("乙")],
        day(16),
    )
    .await
    .expect("都不換股");
    assert_eq!(idle.queries.lock().expect("鎖未中毒").len(), 2);

    let failing = FakeGateway {
        fail: true,
        ..gateway(None)
    };
    let error = notify(
        &failing,
        &recorder,
        &FakeFundamentals::default(),
        &[strategy("甲"), strategy("乙")],
        day(16),
    )
    .await
    .expect_err("回測失敗");
    let message = format!("{error:#}");
    assert!(
        message.contains("甲：") && message.contains("乙："),
        "{message}"
    );
    assert_eq!(failing.queries.lock().expect("鎖未中毒").len(), 2);

    notify(&idle, &recorder, &FakeFundamentals::default(), &[], day(16))
        .await
        .expect("沒有策略");
    // 都不是換股日（或失敗），不寫換股紀錄。
    assert!(recorder.saved.lock().expect("鎖未中毒").is_empty());
}

/// 今天是換股日才發；最後一期已滿或換股日不是今天都不發；回測錯誤往上傳。
#[tokio::test]
async fn run_only_alerts_on_rebalance_day() {
    let fake = gateway(Some(rebalance()));
    let message = rebalance_today(&fake, "from=2019-01-02", day(16))
        .await
        .expect("回測成功");
    assert!(message.is_some_and(|rebalance| rebalance.added[0].symbol == "2330"));
    assert_eq!(
        fake.queries.lock().expect("鎖未中毒").as_slice(),
        ["from=2019-01-02"]
    );

    let message = rebalance_today(&fake, "from=2019-01-02", day(17))
        .await
        .expect("回測成功");
    assert!(message.is_none());

    let none = gateway(None);
    assert!(
        rebalance_today(&none, "from=2019-01-02", day(16))
            .await
            .expect("回測成功")
            .is_none()
    );

    let failing = FakeGateway {
        fail: true,
        ..gateway(None)
    };
    assert!(rebalance_today(&failing, "from=x", day(16)).await.is_err());
}

/// 換股日：寫入紀錄、依策略名稱結算上一期後發通知；結算失敗只略過結算，不影響通知與其他策略。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn notify_settles_previous_period_on_rebalance_day() {
    dotenvy::dotenv().ok();
    let strategy = AlertStrategy {
        name: "甲".to_owned(),
        query: "from=2019-01-02".to_owned(),
    };
    let recorder = FakeRecorder::default();
    let settled = FakeGateway {
        settlement: Some(settlement(50)),
        ..gateway(Some(rebalance()))
    };
    notify(
        &settled,
        &recorder,
        &FakeFundamentals::default(),
        std::slice::from_ref(&strategy),
        day(16),
    )
    .await
    .expect("換股通知成功");
    assert_eq!(
        settled.settled.lock().expect("鎖未中毒").as_slice(),
        [("甲".to_owned(), day(16))]
    );

    let failing = FakeGateway {
        settlement_fail: true,
        ..gateway(Some(rebalance()))
    };
    notify(
        &failing,
        &recorder,
        &FakeFundamentals::failing(),
        &[strategy],
        day(16),
    )
    .await
    .expect("結算失敗不影響通知");
    assert_eq!(failing.settled.lock().expect("鎖未中毒").len(), 1);
    assert_eq!(recorder.saved.lock().expect("鎖未中毒").len(), 2);
}

/// 策略持股有基本面警示時列一行（新買進在前），附名稱與警示；賣出的股票與沒有警示的不列。
#[test]
fn build_message_lists_fundamental_warnings() {
    let warned = |symbol: &str, signals: FundamentalSignals| HoldingFundamentals {
        stock_symbol: symbol.to_owned(),
        signals,
        ..Default::default()
    };
    let fundamentals = [
        warned(
            "2317",
            FundamentalSignals {
                revenue_decline_months: Some(4),
                operating_margin: Some(-1.0),
                ..Default::default()
            },
        ),
        warned(
            "2330",
            FundamentalSignals {
                margin_changes: [Some(-1.0), Some(-1.0), Some(-1.0)],
                ..Default::default()
            },
        ),
        warned(
            "79979",
            FundamentalSignals {
                operating_margin: Some(-5.0),
                ..Default::default()
            },
        ),
    ];
    let message = build_message(&rebalance(), "", None, None, &fundamentals);
    assert!(
        message.contains(&crate::core::util::text::escape_markdown_v2(
            "• 基本面警示：2330 台積電（三率三降）、2317 鴻海（營收連續年減、營業虧損）"
        )),
        "{message}"
    );
    assert!(!build_message(&rebalance(), "", None, None, &[]).contains("基本面警示"));
}
