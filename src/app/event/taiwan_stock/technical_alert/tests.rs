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

/// 回傳固定訊號與歷史表現、記錄查詢條件的假倉儲；`*_fail` 時回錯誤。
#[derive(Default)]
struct FakeQuery {
    signals: Vec<StockTechnicalSignal>,
    fail: bool,
    got: Mutex<Option<(NaiveDate, Vec<String>)>>,
    outcomes: Vec<SignalOutcome>,
    outcomes_fail: bool,
    got_outcomes: Mutex<Option<(NaiveDate, NaiveDate, Vec<String>)>>,
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

/// 相對強度穿越與其他訊號一樣附歷史表現，排在均線交叉之後。
#[test]
fn build_message_lists_rs_crossings_with_outcomes() {
    let signals = notable(vec![
        signal(
            "2330",
            "rs80",
            SignalKind::CrossBelow,
            "相對強度 80 → 79 分，跌破 80 分（第 21 名／101 檔）",
        ),
        signal("2330", "close_ma60", SignalKind::CrossBelow, "跌破季線"),
        signal(
            "0050",
            "rs50",
            SignalKind::CrossAbove,
            "相對強度 40 → 99 分，突破 50 分（第 1 名／101 檔）",
        ),
    ]);
    let outcomes = [outcome("rs80", "cross_below", 300)];
    let message = build_message(date(), &signals, &outcomes, |symbol| match symbol {
        "2330" => "台積電".to_owned(),
        _ => "元大台灣50".to_owned(),
    })
    .expect("有訊號");
    assert_eq!(
        message,
        "持股技術訊號︰2026\\-10\\-13\n\n0050 元大台灣50\n• 相對強度 40 → 99 分，突破 50 分（第 1 名／101 檔）\n\n\
         2330 台積電\n• 跌破季線\n• 相對強度 80 → 79 分，跌破 80 分（第 21 名／101 檔）（20 日勝率 45%、平均 \\-1\\.2%，300 次）\n\n\
         ＊括號內為近一年全市場同一訊號出現後 20 個交易日的還原報酬，過去表現不代表未來結果\n"
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
        signal("2330", "rs50", SignalKind::CrossAbove, "RS 突破 50"),
    ]);
    let texts: Vec<(&str, &str)> = picked
        .iter()
        .map(|s| (s.stock_symbol.as_str(), s.signal.description.as_str()))
        .collect();
    assert_eq!(
        texts,
        [
            ("0050", "死亡交叉"),
            ("2330", "新高"),
            ("2330", "站上季線"),
            ("2330", "RS 突破 50")
        ]
    );
}

/// 同一檔的多個訊號列在同一段，文字經 MarkdownV2 跳脫。
/// 區間突破也通知，排在 52 週新高之後、均線之前，與同一檔的其他訊號併在同一段。
#[test]
fn range_breakout_is_notified_with_other_signals() {
    let picked = notable(vec![
        signal("2330", "close_ma60", SignalKind::CrossAbove, "站上季線"),
        signal("2330", "range_breakout", SignalKind::Breakout, "帶量突破"),
        signal("2330", "52_week", SignalKind::NewHigh, "52 週新高"),
    ]);
    let order: Vec<&str> = picked.iter().map(|s| s.signal.indicator).collect();
    assert_eq!(order, ["52_week", "range_breakout", "close_ma60"]);

    let message = build_message(date(), &picked, &[], |_| "台積電".to_owned()).expect("有訊號");
    assert_eq!(message.matches("台積電").count(), 1, "{message}");
    assert!(message.contains("帶量突破"), "{message}");
}

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
    let message = build_message(date(), &signals, &[], |symbol| match symbol {
        "2330" => "台積電".to_owned(),
        _ => "元大台灣50".to_owned(),
    })
    .expect("有訊號");
    assert_eq!(
        message,
        "持股技術訊號︰2026\\-10\\-13\n\n0050 元大台灣50\n• 均線死亡交叉\n\n2330 台積電\n• 最高價 2600\\.5 創新高\n• 站上季線\n"
    );
    assert!(build_message(date(), &[], &[], |_| String::new()).is_none());
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
        build_message(date(), &signals, &outcomes, |_| "台積電".to_owned()).expect("有訊號");
    assert!(
        message.contains(r"• 跌破年線（20 日勝率 45%、平均 \-1\.2%，1234 次）"),
        "{message}"
    );
    assert!(message.contains("• 跌破季線\n"), "{message}");
    assert!(message.contains("• 均線黃金交叉\n"), "{message}");
    assert!(message.ends_with("過去表現不代表未來結果\n"), "{message}");

    let plain = build_message(date(), &signals[1..], &outcomes, |_| String::new()).expect("有訊號");
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
