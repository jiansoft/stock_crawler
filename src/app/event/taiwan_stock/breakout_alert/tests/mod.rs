//! 全市場區間突破通知的單元與整合測試。

use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;
use crate::domain::quote::technical::StockTechnicalSignal;

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 30).expect("有效日期")
}

fn ranked(symbol: &str, name: &str, industry: &str) -> RankedSignal {
    RankedSignal {
        stock_symbol: symbol.to_owned(),
        name: name.to_owned(),
        industry: industry.to_owned(),
        trade_value: 1e9,
        description: "收盤 10.3 帶量突破 30 日整理區間高點 10".to_owned(),
    }
}

fn outcome(samples: i64) -> SignalOutcome {
    SignalOutcome {
        indicator: RANGE_BREAKOUT_INDICATOR.to_owned(),
        kind: "breakout".to_owned(),
        samples,
        avg_return: 1.23,
        win_rate: 0.52,
    }
}

/// 回傳固定排行與歷史表現、記錄查詢條件的假倉儲。
#[derive(Default)]
struct FakeQuery {
    signals: Vec<RankedSignal>,
    total: i64,
    fail: bool,
    outcomes: Vec<SignalOutcome>,
    outcomes_fail: bool,
    follow_ups: Vec<BreakoutFollowUp>,
    follow_ups_fail: bool,
    got: Mutex<Option<(NaiveDate, String, SignalKind, i64)>>,
}

#[async_trait]
impl BreakoutFollowUpQuery for FakeQuery {
    async fn fetch_breakout_follow_ups(
        &self,
        _date: NaiveDate,
        days: i64,
    ) -> Result<Vec<BreakoutFollowUp>> {
        assert_eq!(days, 5);
        if self.follow_ups_fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok(self.follow_ups.clone())
    }
}

fn follow_up(symbol: &str, name: &str, failed: bool) -> BreakoutFollowUp {
    BreakoutFollowUp {
        stock_symbol: symbol.to_owned(),
        name: name.to_owned(),
        breakout_date: NaiveDate::from_ymd_opt(2026, 10, 27).expect("有效日期"),
        failed,
    }
}

#[async_trait]
impl RankedSignalQuery for FakeQuery {
    async fn fetch_ranked_signals(
        &self,
        date: NaiveDate,
        indicator: &str,
        kind: SignalKind,
        limit: i64,
    ) -> Result<(Vec<RankedSignal>, i64)> {
        *self.got.lock().expect("測試鎖") = Some((date, indicator.to_owned(), kind, limit));
        if self.fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok((self.signals.clone(), self.total))
    }
}

#[async_trait]
impl TechnicalSignalQuery for FakeQuery {
    async fn fetch_signals(
        &self,
        _date: NaiveDate,
        _stock_symbols: &[String],
    ) -> Result<Vec<StockTechnicalSignal>> {
        Ok(Vec::new())
    }

    async fn fetch_signal_outcomes(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
        _indicators: &[&str],
    ) -> Result<Vec<SignalOutcome>> {
        if self.outcomes_fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok(self.outcomes.clone())
    }
}

/// 標題附總檔數與列出檔數；產業、持股標記與勝率都有；結尾附回測結論與免責。
#[test]
fn build_message_lists_signals_with_holding_mark_and_outcome() {
    let signals = [
        ranked("2330", "台積電", "半導體業"),
        ranked("9999", "測試", ""),
    ];
    let holdings = BTreeSet::from(["2330".to_owned()]);

    let message = build_message(date(), &signals, 37, Some(&outcome(120)), &[], &holdings);

    for want in [
        "區間突破︰2026\\-10\\-30（全市場 37 檔，依成交值列前 2）",
        "2330 台積電（半導體業）【持股】",
        "9999 測試\n",
        "• 收盤 10\\.3 帶量突破 30 日整理區間高點 10",
        "勝率 52%、平均還原報酬 \\+1\\.2%（120 次）",
        "單獨依區間突破選股明顯輸 0050",
    ] {
        assert!(message.contains(want), "缺少 {want:?}：{message}");
    }
    assert!(!message.contains("9999 測試【持股】"), "{message}");
}

/// 沒有歷史表現時只附回測結論。
#[test]
fn build_message_without_outcome() {
    let message = build_message(
        date(),
        &[ranked("2330", "台積電", "")],
        1,
        None,
        &[],
        &BTreeSet::new(),
    );

    assert!(!message.contains("勝率"), "{message}");
    assert!(!message.contains("前 5 個交易日"), "{message}");
    assert!(message.contains("不構成投資建議"), "{message}");
}

/// 查詢條件：當天、range_breakout／breakout、前 15 檔；樣本不足或假突破的歷史表現不附。
#[tokio::test]
async fn run_queries_top_signals_and_skips_small_samples() {
    let repo = FakeQuery {
        signals: vec![ranked("2330", "台積電", "半導體業")],
        total: 5,
        outcomes: vec![
            outcome(10),
            SignalOutcome {
                kind: "failed".to_owned(),
                ..outcome(500)
            },
        ],
        ..Default::default()
    };

    let message = run(&repo, date(), &BTreeSet::new())
        .await
        .expect("查詢成功")
        .expect("有訊號");

    assert_eq!(
        repo.got.lock().expect("測試鎖").take(),
        Some((
            date(),
            "range_breakout".to_owned(),
            SignalKind::Breakout,
            15
        ))
    );
    assert!(!message.contains("勝率"), "{message}");
}

/// 沒有訊號（含休市日）不發；查詢失敗回錯誤；歷史表現讀取失敗照常通知。
#[tokio::test]
async fn run_handles_empty_failure_and_outcome_failure() {
    let empty = FakeQuery::default();
    assert_eq!(
        run(&empty, date(), &BTreeSet::new())
            .await
            .expect("查詢成功"),
        None
    );

    let failing = FakeQuery {
        fail: true,
        ..Default::default()
    };
    assert!(run(&failing, date(), &BTreeSet::new()).await.is_err());

    let outcome_failing = FakeQuery {
        signals: vec![ranked("2330", "台積電", "")],
        total: 1,
        outcomes_fail: true,
        ..Default::default()
    };
    let message = run(&outcome_failing, date(), &BTreeSet::new())
        .await
        .expect("照常通知")
        .expect("有訊號");
    assert!(message.contains("2330 台積電"), "{message}");
}

/// 前幾日突破的後續：總檔數、仍站上比例與已跌回檔數；持股各自的狀態；沒有突破時不附。
#[test]
fn follow_up_lines_count_held_and_failed() {
    assert!(follow_up_lines(&[], &BTreeSet::new()).is_empty());

    let follow_ups = [
        follow_up("2330", "台積電", true),
        follow_up("2317", "鴻海", false),
        follow_up("9999", "測試", true),
        follow_up("0050", "元大台灣50", false),
    ];
    assert_eq!(
        follow_up_lines(&follow_ups, &BTreeSet::new()),
        ["前 5 個交易日突破 4 檔：仍站上突破價 2 檔（50%）、已跌回 2 檔"]
    );

    let holdings = BTreeSet::from(["2330".to_owned(), "0050".to_owned()]);
    assert_eq!(
        follow_up_lines(&follow_ups, &holdings),
        [
            "前 5 個交易日突破 4 檔：仍站上突破價 2 檔（50%）、已跌回 2 檔",
            "持股：2330 台積電 已跌回、0050 元大台灣50 仍站上",
        ]
    );
}

/// 後續一段放在個股清單之後、結尾說明之前，並經 MarkdownV2 跳脫。
#[test]
fn build_message_appends_follow_ups_before_footer() {
    let lines = vec!["前 5 個交易日突破 4 檔：仍站上突破價 2 檔（50%）、已跌回 2 檔".to_owned()];
    let message = build_message(
        date(),
        &[ranked("2330", "台積電", "")],
        1,
        None,
        &lines,
        &BTreeSet::new(),
    );

    let follow = message
        .find("• 前 5 個交易日突破 4 檔：仍站上突破價 2 檔（50%）、已跌回 2 檔")
        .expect("有後續一段");
    let footer = message.find("＊2019 年起回測").expect("有結尾說明");
    assert!(follow < footer, "{message}");
}

/// 通知帶上前幾日突破的後續；後續查詢失敗只是不附，照常通知。
#[tokio::test]
async fn run_appends_follow_ups_and_tolerates_failure() {
    let repo = FakeQuery {
        signals: vec![ranked("2330", "台積電", "")],
        total: 1,
        follow_ups: vec![follow_up("2317", "鴻海", true)],
        ..Default::default()
    };
    let holdings = BTreeSet::from(["2317".to_owned()]);
    let message = run(&repo, date(), &holdings)
        .await
        .expect("查詢成功")
        .expect("有訊號");
    assert!(
        message.contains("前 5 個交易日突破 1 檔：仍站上突破價 0 檔（0%）、已跌回 1 檔"),
        "{message}"
    );
    assert!(message.contains("持股：2317 鴻海 已跌回"), "{message}");

    let failing = FakeQuery {
        signals: vec![ranked("2330", "台積電", "")],
        total: 1,
        follow_ups_fail: true,
        ..Default::default()
    };
    let message = run(&failing, date(), &holdings)
        .await
        .expect("照常通知")
        .expect("有訊號");
    assert!(!message.contains("前 5 個交易日"), "{message}");
}

mod database;
