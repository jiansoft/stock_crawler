//! # 策略換股通知
//!
//! 每個交易日 22:10（籌碼 21:50 入庫之後），對每個 `alert` 為 true 的具名策略（`strategy_preset`，
//! 條件與 `/stocks/screen/backtest` 的 query string 相同，必須指定 `from`，換股日從起日起算）各跑一次
//! 回測；最後一次換股是今天時，用 Telegram 通知新買進、續抱與賣出的股票與權重，並對照目前的實際持股
//! （策略持股中已持有、尚未持有的股票，策略賣出但仍持有的股票，以及 0050 核心部位）。`.env` 的
//! [`QUERY_ENV`]／[`NAME_ENV`] 仍可另設一組（與具名策略條件相同時不重複通知）。都沒有就不做事；
//! 讀不到實際持股時只略過對照；某個策略失敗時其餘照常通知，最後回傳錯誤。
//!
//! 換股日另把當天的持股寫進 `strategy_rebalance`（見 [`rebalance_record`]），供
//! `/strategy-presets/{name}/track` 追蹤實際表現；寫入失敗只記 warning，通知照發。
//!
//! 回測引擎在 `interfaces::web::data_api`，這裡只透過 [`ports::StrategyGateway`] 取得換股結果。

use std::collections::BTreeSet;
use std::fmt::Write;

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDate};

use super::holdings;
use crate::{
    app::ports::{self, StrategyGateway, StrategyHolding, StrategyRebalance},
    core::{alert, util::text},
    domain::strategy::{
        StrategyPreset, StrategyPresetRepository,
        rebalance::{RecordedHolding, StrategyRebalanceRecord, StrategyRebalanceRepository},
    },
    infra::database::repository::{
        strategy_preset::PgStrategyPresetRepository,
        strategy_rebalance::PgStrategyRebalanceRepository,
    },
};

/// 另一組策略條件的環境變數（條件選股回測的 query string）；沒有設定或空白時只看具名策略。
pub const QUERY_ENV: &str = "STRATEGY_ALERT_QUERY";

/// 通知標題中的策略名稱環境變數；未設定時只寫「策略換股」。
pub const NAME_ENV: &str = "STRATEGY_ALERT_NAME";

/// 要通知的一個策略：名稱（通知標題用，可為空）與條件。
#[derive(Debug, Clone, PartialEq, Eq)]
struct AlertStrategy {
    name: String,
    query: String,
}

/// 要通知的策略：`alert` 為 true 的具名策略（依名稱），再加上環境變數的那一組（條件空白或與某個
/// 具名策略相同時不加）。
fn alert_strategies(
    presets: Vec<StrategyPreset>,
    env_query: Option<String>,
    env_name: Option<String>,
) -> Vec<AlertStrategy> {
    let mut strategies: Vec<AlertStrategy> = presets
        .into_iter()
        .filter(|preset| preset.alert)
        .map(|preset| AlertStrategy {
            name: preset.name,
            query: preset.query.trim().to_owned(),
        })
        .collect();
    let query = env_query.unwrap_or_default().trim().to_owned();
    if !query.is_empty() && !strategies.iter().any(|strategy| strategy.query == query) {
        strategies.push(AlertStrategy {
            name: env_name.unwrap_or_default().trim().to_owned(),
            query,
        });
    }
    strategies
}

/// 排程入口：對每個要通知的策略，今天是換股日時發通知。
///
/// # Errors
///
/// 讀不到具名策略、策略 gateway 未註冊，或有策略的回測條件錯誤或回測失敗時回傳錯誤（其餘策略照常
/// 通知）。
pub async fn execute() -> Result<()> {
    let presets = PgStrategyPresetRepository::new()
        .list()
        .await
        .context("讀取具名策略失敗")?;
    let strategies = alert_strategies(
        presets,
        std::env::var(QUERY_ENV).ok(),
        std::env::var(NAME_ENV).ok(),
    );
    if strategies.is_empty() {
        tracing::info!("策略換股通知：沒有要通知的具名策略，也未設定 {QUERY_ENV}，略過");
        return Ok(());
    }
    let gateway = ports::strategy_gateway()?;
    notify(
        gateway.as_ref(),
        &PgStrategyRebalanceRepository::new(),
        &strategies,
        Local::now().date_naive(),
    )
    .await
}

/// 手動回補用：以具名策略目前的條件跑一次回測，把最後一次換股（還沒滿持有天數的那一期）寫進換股
/// 紀錄，回傳換股日與是否新寫入；策略不存在回錯誤，最後一期剛好滿持有天數回 `None`。
///
/// 換股通知啟用前的那一期沒有被記錄，用這個補上，之後的換股由 22:10 的通知照常記錄。
///
/// # Errors
///
/// 讀不到策略、策略 gateway 未註冊、回測失敗或寫入失敗時回傳錯誤。
pub async fn record_current_rebalance(preset_name: &str) -> Result<Option<(NaiveDate, bool)>> {
    let preset = PgStrategyPresetRepository::new()
        .find(preset_name)
        .await
        .context("讀取具名策略失敗")?
        .with_context(|| format!("查無具名策略：{preset_name}"))?;
    let strategy = AlertStrategy {
        name: preset.name,
        query: preset.query.trim().to_owned(),
    };
    let Some(rebalance) = ports::strategy_gateway()?
        .latest_rebalance(&strategy.query)
        .await?
    else {
        return Ok(None);
    };
    let record = rebalance_record(&strategy, &rebalance);
    let inserted = PgStrategyRebalanceRepository::new().save(&record).await?;
    Ok(Some((record.date, inserted)))
}

/// 逐一跑策略回測，今天換股的寫入換股紀錄並發通知；實際持股只在第一次需要時讀一次。
async fn notify(
    gateway: &dyn StrategyGateway,
    recorder: &dyn StrategyRebalanceRepository,
    strategies: &[AlertStrategy],
    today: NaiveDate,
) -> Result<()> {
    let mut held: Option<Option<BTreeSet<String>>> = None;
    let mut failures: Vec<String> = Vec::new();
    for strategy in strategies {
        let rebalance = match rebalance_today(gateway, &strategy.query, today).await {
            Ok(Some(rebalance)) => rebalance,
            Ok(None) => continue,
            Err(error) => {
                tracing::error!("策略換股通知：{} 失敗：{error:?}", strategy.name);
                failures.push(format!("{}：{error:#}", strategy.name));
                continue;
            }
        };
        let record = rebalance_record(strategy, &rebalance);
        match recorder.save(&record).await {
            Ok(true) => tracing::info!(
                "策略換股通知：已記錄 {} {} 的換股",
                record.preset_name,
                record.date
            ),
            Ok(false) => tracing::info!(
                "策略換股通知：{} {} 已有換股紀錄，保留原本的",
                record.preset_name,
                record.date
            ),
            Err(error) => tracing::warn!(
                "策略換股通知：寫入 {} 換股紀錄失敗：{error:?}",
                record.preset_name
            ),
        }
        if held.is_none() {
            held = Some(match holdings::active_holding_symbols().await {
                Ok(held) => Some(held),
                Err(error) => {
                    tracing::warn!("策略換股通知：讀取實際持股失敗，略過對照：{error:?}");
                    None
                }
            });
        }
        let actual = held.as_ref().and_then(Option::as_ref);
        alert::send_message(&build_message(&rebalance, &strategy.name, actual)).await;
    }
    if !failures.is_empty() {
        bail!("策略換股通知失敗：{}", failures.join("；"));
    }
    Ok(())
}

/// 沒有名稱（只設 [`QUERY_ENV`]）的策略在換股紀錄中的名稱。
const UNNAMED_STRATEGY: &str = "STRATEGY_ALERT_QUERY";

/// 換股紀錄：策略部位的新買進（在前）與續抱持股及權重；賣出的不記。
fn rebalance_record(
    strategy: &AlertStrategy,
    rebalance: &StrategyRebalance,
) -> StrategyRebalanceRecord {
    StrategyRebalanceRecord {
        preset_name: if strategy.name.is_empty() {
            UNNAMED_STRATEGY.to_owned()
        } else {
            strategy.name.clone()
        },
        date: rebalance.date,
        query: strategy.query.clone(),
        hold_days: rebalance.hold_days,
        etf_weight: rebalance.etf_weight,
        cash: rebalance.cash,
        market_filtered: rebalance.market_filtered,
        holdings: rebalance
            .added
            .iter()
            .chain(&rebalance.kept)
            .map(|holding| RecordedHolding {
                symbol: holding.symbol.clone(),
                name: holding.name.clone(),
                weight: holding.weight,
            })
            .collect(),
    }
}

/// 跑一次策略回測，取今天的換股；最後一次換股不是 `today`（今天不是換股日或休市）時回 `None`。
async fn rebalance_today(
    gateway: &dyn StrategyGateway,
    query: &str,
    today: NaiveDate,
) -> Result<Option<StrategyRebalance>> {
    let Some(rebalance) = gateway.latest_rebalance(query).await? else {
        tracing::info!("策略換股通知：最後一期已滿持有天數，今天不是換股日，略過");
        return Ok(None);
    };
    if rebalance.date != today {
        tracing::info!(
            "策略換股通知：最後一次換股為 {}，不是今天 {today}，略過",
            rebalance.date
        );
        return Ok(None);
    }
    Ok(Some(rebalance))
}

/// 代號與名稱；沒有名稱時只有代號。
fn symbol_text(holding: &StrategyHolding) -> String {
    match holding.name.as_deref() {
        Some(name) if !name.is_empty() => format!("{} {name}", holding.symbol),
        _ => holding.symbol.clone(),
    }
}

/// 對照實際持股的幾行：策略持股中已持有與尚未持有的股票、策略賣出但仍持有的股票，以及有 0050
/// 核心部位時是否持有 0050。
fn holdings_lines(rebalance: &StrategyRebalance, held: &BTreeSet<String>) -> Vec<String> {
    let strategy: Vec<&StrategyHolding> = rebalance.added.iter().chain(&rebalance.kept).collect();
    let (owned, missing): (Vec<&StrategyHolding>, Vec<&StrategyHolding>) = strategy
        .iter()
        .partition(|holding| held.contains(&holding.symbol));
    let list = |items: &[&StrategyHolding]| {
        items
            .iter()
            .map(|holding| symbol_text(holding))
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = Vec::new();
    if !strategy.is_empty() {
        let mut line = format!(
            "• 對照實際持股：策略 {} 檔中已持有 {} 檔",
            strategy.len(),
            owned.len()
        );
        if !missing.is_empty() {
            let _ = write!(&mut line, "；尚未持有：{}", list(&missing));
        }
        lines.push(line);
    }
    let still_held: Vec<&StrategyHolding> = rebalance
        .exited
        .iter()
        .filter(|holding| held.contains(&holding.symbol))
        .collect();
    if !still_held.is_empty() {
        lines.push(format!("• 策略賣出但仍持有：{}", list(&still_held)));
    }
    if rebalance.etf_weight > 0 {
        let status = if held.contains("0050") {
            "已持有"
        } else {
            "尚未持有"
        };
        lines.push(format!("• 0050 核心部位：{status}"));
    }
    lines
}

/// 一檔股票：代號、名稱與權重。
fn holding_text(holding: &StrategyHolding) -> String {
    let name = holding
        .name
        .as_deref()
        .map(|name| format!(" {name}"))
        .unwrap_or_default();
    format!("{}{name} {:.1}%", holding.symbol, holding.weight)
}

/// 一類股票的一行；沒有股票時回 `None`。
fn group_line(label: &str, holdings: &[StrategyHolding]) -> Option<String> {
    if holdings.is_empty() {
        return None;
    }
    let list: Vec<String> = holdings.iter().map(holding_text).collect();
    Some(format!(
        "• {label}（{} 檔）：{}",
        holdings.len(),
        list.join("、")
    ))
}

/// 組出換股通知（MarkdownV2 已跳脫）；`actual` 為目前的實際持股代號，`None` 時不對照。
fn build_message(
    rebalance: &StrategyRebalance,
    name: &str,
    actual: Option<&BTreeSet<String>>,
) -> String {
    let title = if name.is_empty() {
        format!("策略換股︰{}", rebalance.date)
    } else {
        format!("策略換股（{name}）︰{}", rebalance.date)
    };
    let held = rebalance.added.len() + rebalance.kept.len();
    let mut lines = Vec::new();
    if rebalance.market_filtered {
        lines.push("• 大盤濾網沒過，策略部位全部轉為現金".to_owned());
    } else if held == 0 {
        lines.push("• 沒有符合條件的股票，策略部位全部轉為現金".to_owned());
    } else if rebalance.cash > 0.0 {
        lines.push(format!(
            "• 持有 {held} 檔，策略部位現金 {:.1}%（單檔權重上限）",
            rebalance.cash
        ));
    } else {
        lines.push(format!("• 持有 {held} 檔"));
    }
    lines.extend(group_line("新買進", &rebalance.added));
    lines.extend(group_line("續抱", &rebalance.kept));
    lines.extend(group_line("賣出", &rebalance.exited));
    let mut summary = format!("• 換手 {:.1}%", rebalance.turnover * 100.0);
    if rebalance.etf_weight > 0 {
        let _ = write!(
            &mut summary,
            "；0050 核心部位 {}%（權重為策略部位內的比例，0050 照常持有）",
            rebalance.etf_weight
        );
    }
    lines.push(summary);
    lines.push(format!("• 下次換股：{} 個交易日後", rebalance.hold_days));
    if let Some(actual) = actual {
        lines.extend(holdings_lines(rebalance, actual));
    }

    let mut message = String::with_capacity(512);
    let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(title));
    for line in lines {
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    let _ = writeln!(
        &mut message,
        "\n{}",
        text::escape_markdown_v2(
            "＊依回測條件以今天收盤選出，實際下單在下一個交易日；過去表現不代表未來結果"
        )
    );
    message
}

#[cfg(test)]
pub(in crate::app::event::taiwan_stock) mod tests {
    use std::sync::Mutex;

    use anyhow::bail;
    use async_trait::async_trait;

    use super::*;

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

    /// 回傳固定結果並記下收到的條件。
    struct FakeGateway {
        result: Option<StrategyRebalance>,
        fail: bool,
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
    }

    fn gateway(result: Option<StrategyRebalance>) -> FakeGateway {
        FakeGateway {
            result,
            fail: false,
            queries: Mutex::new(Vec::new()),
        }
    }

    /// 新買進、續抱、賣出各一行，附換手與下次換股。
    #[test]
    fn build_message_lists_changes() {
        let message = build_message(&rebalance(), "動能", None);
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
        let message = build_message(&filtered, "", None);
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
        assert!(build_message(&empty, "", None).contains("沒有符合條件的股票"));

        let capped = StrategyRebalance {
            cash: 40.0,
            ..rebalance()
        };
        assert!(
            build_message(&capped, "", None).contains("策略部位現金 40\\.0%"),
            "{}",
            build_message(&capped, "", None)
        );
    }

    /// 對照實際持股：已持有與尚未持有、策略賣出但仍持有、0050 核心部位；讀不到持股時不列。
    #[test]
    fn build_message_compares_with_actual_holdings() {
        let held: BTreeSet<String> = ["2317", "79979", "0050"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let message = build_message(&rebalance(), "", Some(&held));
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
        let message = build_message(&core, "", Some(&held));
        assert!(message.contains("• 0050 核心部位：已持有"), "{message}");
        assert!(!message.contains("仍持有"), "{message}");
        let all: BTreeSet<String> = ["2330", "2317"].into_iter().map(str::to_owned).collect();
        let message = build_message(&core, "", Some(&all));
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
        let message = build_message(&filtered, "", Some(&held));
        assert!(!message.contains("對照實際持股"), "{message}");
        assert!(message.contains("策略賣出但仍持有：79979"), "{message}");
        assert!(!build_message(&rebalance(), "", None).contains("對照實際持股"));
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
        notify(&idle, &recorder, &[strategy("甲"), strategy("乙")], day(16))
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

        notify(&idle, &recorder, &[], day(16))
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
}
