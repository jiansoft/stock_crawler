//! 持股技術訊號通知的單元測試。

use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;
use crate::domain::quote::technical::{SignalKind, TechnicalSignal};

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 13).expect("有效日期")
}

fn signal(
    symbol: &str,
    indicator: &'static str,
    kind: SignalKind,
    text: &str,
) -> StockTechnicalSignal {
    StockTechnicalSignal {
        date: date(),
        stock_symbol: symbol.to_owned(),
        signal: TechnicalSignal {
            indicator,
            kind,
            description: text.to_owned(),
        },
    }
}

/// 回傳固定訊號、歷史表現與相對強度、記錄查詢條件的假倉儲；`*_fail` 時回錯誤。
#[derive(Default)]
struct FakeQuery {
    signals: Vec<StockTechnicalSignal>,
    fail: bool,
    got: Mutex<Option<(NaiveDate, Vec<String>)>>,
    outcomes: Vec<SignalOutcome>,
    outcomes_fail: bool,
    got_outcomes: Mutex<Option<(NaiveDate, NaiveDate, Vec<String>)>>,
    rs: Vec<RelativeStrengthDay>,
    rs_fail: bool,
    got_rs: Mutex<Option<(NaiveDate, u16, Vec<String>)>>,
}

/// universe 101 時分數＝weaker × 98 ÷ 100 取整＋1（weaker 81 → 80、80 → 79）。
fn rs_day(symbol: &str, day: u32, weaker: i64) -> RelativeStrengthDay {
    RelativeStrengthDay {
        stock_symbol: symbol.to_owned(),
        date: NaiveDate::from_ymd_opt(2026, 10, day).expect("有效日期"),
        weighted: 1.0,
        rs_score: crate::domain::quote::relative_strength::percentile_score(
            usize::try_from(weaker).expect("測試資料非負"),
            101,
        ),
        rank: 101 - weaker,
        universe: 101,
    }
}

fn outcome(indicator: &str, kind: &str, samples: i64) -> SignalOutcome {
    SignalOutcome {
        indicator: indicator.to_owned(),
        kind: kind.to_owned(),
        samples,
        avg_return: -1.234,
        win_rate: 0.4523,
    }
}

#[async_trait]
impl TechnicalSignalQuery for FakeQuery {
    async fn fetch_signals(
        &self,
        date: NaiveDate,
        stock_symbols: &[String],
    ) -> Result<Vec<StockTechnicalSignal>> {
        *self.got.lock().expect("測試鎖") = Some((date, stock_symbols.to_vec()));
        if self.fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok(self.signals.clone())
    }

    async fn fetch_signal_outcomes(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        indicators: &[&str],
    ) -> Result<Vec<SignalOutcome>> {
        *self.got_outcomes.lock().expect("測試鎖") = Some((
            from,
            to,
            indicators.iter().map(|s| (*s).to_owned()).collect(),
        ));
        if self.outcomes_fail {
            return Err(anyhow!("統計逾時"));
        }
        Ok(self.outcomes.clone())
    }
}

#[async_trait]
impl RelativeStrengthQuery for FakeQuery {
    async fn fetch_relative_strength(
        &self,
        until: NaiveDate,
        days: u16,
        stock_symbols: &[String],
    ) -> Result<Vec<RelativeStrengthDay>> {
        *self.got_rs.lock().expect("測試鎖") = Some((until, days, stock_symbols.to_vec()));
        if self.rs_fail {
            return Err(anyhow!("RS 逾時"));
        }
        Ok(self.rs.clone())
    }
}

/// 只列當天相對前一個交易日穿越門檻的持股；同一天跨兩個門檻合併成一筆；當天沒有分數
/// （報酬還沒回填）或沒有前一天分數的不列。
#[test]
fn rs_turns_detect_crossings() {
    let days = [
        rs_day("0050", 13, 100),
        rs_day("0050", 12, 40),
        rs_day("2317", 13, 60),
        rs_day("2317", 12, 70),
        rs_day("2330", 13, 80),
        rs_day("2330", 12, 81),
        rs_day("2454", 12, 81),
        rs_day("2454", 9, 80),
        rs_day("3008", 13, 81),
    ];
    let turns = rs_turns(&days, date());
    assert_eq!(
        turns,
        [
            RsTurn {
                stock_symbol: "0050".to_owned(),
                previous: 40,
                current: 99,
                rank: 1,
                universe: 101,
                direction: CrossDirection::Up,
                thresholds: vec![80, 50],
            },
            RsTurn {
                stock_symbol: "2330".to_owned(),
                previous: 80,
                current: 79,
                rank: 21,
                universe: 101,
                direction: CrossDirection::Down,
                thresholds: vec![80],
            },
        ]
    );
    assert_eq!(
        turn_text(&turns[0]),
        "相對強度 40 → 99，突破 80、50 分（全體第 1 名／101 檔）"
    );
    assert_eq!(
        turn_text(&turns[1]),
        "相對強度 80 → 79，跌破 80 分（全體第 21 名／101 檔）"
    );
    assert!(rs_turns(&[], date()).is_empty());
}

/// RS 轉折與技術訊號依代號合併在同一段，訊號在前、RS 在後；只有 RS 轉折也會產生訊息。
#[test]
fn build_message_lists_rs_turns() {
    let turns = rs_turns(
        &[
            rs_day("0050", 13, 100),
            rs_day("0050", 12, 40),
            rs_day("2330", 13, 80),
            rs_day("2330", 12, 81),
        ],
        date(),
    );
    let only_rs =
        build_message(date(), &[], &[], &turns[1..], |_| "台積電".to_owned()).expect("有 RS 轉折");
    assert_eq!(
        only_rs,
        "持股技術訊號︰2026\\-10\\-13\n\n2330 台積電\n• 相對強度 80 → 79，跌破 80 分（全體第 21 名／101 檔）\n"
    );

    let signals = [signal(
        "2330",
        "close_ma60",
        SignalKind::CrossBelow,
        "跌破季線",
    )];
    let mixed = build_message(date(), &signals, &[], &turns, |symbol| match symbol {
        "2330" => "台積電".to_owned(),
        _ => "元大台灣50".to_owned(),
    })
    .expect("有訊號");
    assert_eq!(
        mixed,
        "持股技術訊號︰2026\\-10\\-13\n\n0050 元大台灣50\n• 相對強度 40 → 99，突破 80、50 分（全體第 1 名／101 檔）\n\n2330 台積電\n• 跌破季線\n• 相對強度 80 → 79，跌破 80 分（全體第 21 名／101 檔）\n"
    );
}

/// 過濾掉高頻訊號，依代號與通知順序排列。
#[test]
fn notable_keeps_long_term_signals_in_order() {
    let picked = notable(vec![
        signal("2330", "kd", SignalKind::GoldenCross, "KD"),
        signal("2330", "close_ma60", SignalKind::CrossAbove, "站上季線"),
        signal("0050", "ma20_ma60", SignalKind::DeathCross, "死亡交叉"),
        signal("2330", "52_week", SignalKind::NewHigh, "新高"),
        signal("2330", "rsi14", SignalKind::Overbought, "RSI"),
    ]);
    let texts: Vec<(&str, &str)> = picked
        .iter()
        .map(|s| (s.stock_symbol.as_str(), s.signal.description.as_str()))
        .collect();
    assert_eq!(
        texts,
        [("0050", "死亡交叉"), ("2330", "新高"), ("2330", "站上季線")]
    );
}

/// 同一檔的多個訊號列在同一段，文字經 MarkdownV2 跳脫。
#[test]
fn build_message_groups_by_stock() {
    let signals = [
        signal("0050", "ma20_ma60", SignalKind::DeathCross, "均線死亡交叉"),
        signal(
            "2330",
            "52_week",
            SignalKind::NewHigh,
            "最高價 2600.5 創新高",
        ),
        signal("2330", "close_ma60", SignalKind::CrossAbove, "站上季線"),
    ];
    let message = build_message(date(), &signals, &[], &[], |symbol| match symbol {
        "2330" => "台積電".to_owned(),
        _ => "元大台灣50".to_owned(),
    })
    .expect("有訊號");
    assert_eq!(
        message,
        "持股技術訊號︰2026\\-10\\-13\n\n0050 元大台灣50\n• 均線死亡交叉\n\n2330 台積電\n• 最高價 2600\\.5 創新高\n• 站上季線\n"
    );
    assert!(build_message(date(), &[], &[], &[], |_| String::new()).is_none());
}

/// 樣本足夠的訊號附上勝率與平均報酬並加註腳；樣本不足或種類不同的不附。
#[test]
fn build_message_appends_outcomes() {
    let signals = [
        signal("2330", "close_ma240", SignalKind::CrossBelow, "跌破年線"),
        signal("2330", "close_ma60", SignalKind::CrossBelow, "跌破季線"),
        signal("2330", "ma20_ma60", SignalKind::GoldenCross, "均線黃金交叉"),
    ];
    let outcomes = [
        outcome("close_ma240", "cross_below", 1234),
        outcome("close_ma60", "cross_below", MIN_OUTCOME_SAMPLES - 1),
        outcome("ma20_ma60", "death_cross", 500),
    ];
    let message =
        build_message(date(), &signals, &outcomes, &[], |_| "台積電".to_owned()).expect("有訊號");
    assert!(
        message.contains(r"• 跌破年線（20 日勝率 45%、平均 \-1\.2%，1234 次）"),
        "{message}"
    );
    assert!(message.contains("• 跌破季線\n"), "{message}");
    assert!(message.contains("• 均線黃金交叉\n"), "{message}");
    assert!(message.ends_with("過去表現不代表未來結果\n"), "{message}");

    let plain =
        build_message(date(), &signals[1..], &outcomes, &[], |_| String::new()).expect("有訊號");
    assert!(!plain.contains("過去表現"), "沒有附勝率時不加註腳");
    assert_eq!(
        outcome_text(&outcome("kd", "x", 9)),
        "20 日勝率 45%、平均 -1.2%，9 次"
    );
}

/// 以持股代號查當天訊號；沒有要通知的訊號時不產生訊息，查詢失敗回錯誤。
#[tokio::test]
async fn run_queries_holdings_and_builds_message() {
    let symbols: BTreeSet<String> = ["2330".to_owned(), "0050".to_owned()].into();
    let repo = FakeQuery {
        signals: vec![signal(
            "2330",
            "close_ma240",
            SignalKind::CrossBelow,
            "跌破年線",
        )],
        ..Default::default()
    };
    let message = run(&repo, date(), &symbols)
        .await
        .expect("成功")
        .expect("有訊號");
    assert!(message.contains("• 跌破年線"), "{message}");
    assert_eq!(
        repo.got.lock().expect("測試鎖").clone(),
        Some((date(), vec!["0050".to_owned(), "2330".to_owned()]))
    );
    assert_eq!(
        repo.got_outcomes.lock().expect("測試鎖").clone(),
        Some((
            NaiveDate::from_ymd_opt(2025, 10, 14).expect("有效日期"),
            date(),
            NOTIFY_INDICATORS.iter().map(|s| (*s).to_owned()).collect()
        ))
    );
    assert_eq!(
        repo.got_rs.lock().expect("測試鎖").clone(),
        Some((date(), RS_DAYS, vec!["0050".to_owned(), "2330".to_owned()]))
    );

    // 只有 RS 轉折也通知，但不查訊號歷史表現；RS 查詢失敗仍照常通知技術訊號。
    let rs_only = FakeQuery {
        rs: vec![rs_day("2330", 13, 80), rs_day("2330", 12, 81)],
        ..Default::default()
    };
    let message = run(&rs_only, date(), &symbols)
        .await
        .expect("成功")
        .expect("有 RS 轉折");
    assert!(message.contains("跌破 80 分"), "{message}");
    assert!(rs_only.got_outcomes.lock().expect("測試鎖").is_none());
    let rs_failing = FakeQuery {
        signals: vec![signal(
            "2330",
            "close_ma240",
            SignalKind::CrossBelow,
            "跌破年線",
        )],
        rs_fail: true,
        ..Default::default()
    };
    let message = run(&rs_failing, date(), &symbols)
        .await
        .expect("成功")
        .expect("有訊號");
    assert!(message.contains("• 跌破年線"), "{message}");
    assert!(!message.contains("相對強度"), "{message}");

    // 歷史表現查詢失敗仍照常通知，只是不附勝率。
    let degraded = FakeQuery {
        signals: vec![signal(
            "2330",
            "close_ma240",
            SignalKind::CrossBelow,
            "跌破年線",
        )],
        outcomes: vec![outcome("close_ma240", "cross_below", 100)],
        outcomes_fail: true,
        ..Default::default()
    };
    let message = run(&degraded, date(), &symbols)
        .await
        .expect("成功")
        .expect("有訊號");
    assert!(message.contains("• 跌破年線\n"), "{message}");

    let quiet = FakeQuery {
        signals: vec![signal("2330", "kd", SignalKind::GoldenCross, "KD")],
        ..Default::default()
    };
    assert!(run(&quiet, date(), &symbols).await.expect("成功").is_none());
    assert!(
        quiet.got_outcomes.lock().expect("測試鎖").is_none(),
        "沒有要通知的訊號時不查歷史表現"
    );

    let failing = FakeQuery {
        fail: true,
        ..Default::default()
    };
    assert!(run(&failing, date(), &symbols).await.is_err());
}
