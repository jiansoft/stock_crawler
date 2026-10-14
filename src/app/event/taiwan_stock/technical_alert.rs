//! # 持股技術訊號通知
//!
//! 每個交易日晚上讀收盤後預先算好的技術訊號（`daily_technical_signal`，見
//! [`crate::app::calculation::technical_signal`]），列出持股出現的重要訊號。
//!
//! - 持股包含 ETF（例如 0050 創新高、0056 跌破年線也值得知道），不含已售出的持股。
//! - 只通知中長期、出現頻率低的訊號（[`NOTIFY_INDICATORS`]）：52 週新高新低、收盤站上或跌破
//!   季線與年線、20 日與 60 日均線交叉。KD、RSI 超買超賣與短天期均線幾乎天天有，不通知。
//! - 每個訊號附上近一年全市場同一訊號出現後 20 個交易日的勝率與平均還原報酬（樣本至少
//!   [`MIN_OUTCOME_SAMPLES`] 次才顯示）；統計查詢失敗只記錄警告，照常通知。
//! - 另列當天相對強度（RS，見 [`crate::domain::quote::relative_strength`]）穿越 80 或 50 分的
//!   持股（與前一個交易日比較）；當天報酬還沒回填就不列，查詢失敗只記錄警告。
//! - 排程在 19:15（週一至週五），排在 19:00 收盤資料完整性檢查之後，重跑收盤匯總的訊號也已定案。
//! - 沒有任何持股出現訊號或 RS 轉折（含休市日）時不發送。

use std::collections::BTreeSet;
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate};

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::{
        portfolio::repository::PortfolioRepository,
        quote::{
            relative_strength::{
                CrossDirection, RelativeStrengthDay, RelativeStrengthQuery, crossings,
            },
            technical::{
                FIFTY_TWO_WEEK_INDICATOR, OUTCOME_HORIZON, SignalOutcome, StockTechnicalSignal,
                TechnicalSignalQuery,
            },
        },
    },
    infra::database::repository::{portfolio::PgPortfolioRepository, quote::PgQuoteRepository},
};

/// 要通知的訊號來源，也是同一檔股票內訊號的排列順序。
pub const NOTIFY_INDICATORS: [&str; 4] = [
    FIFTY_TWO_WEEK_INDICATOR,
    "close_ma240",
    "close_ma60",
    "ma20_ma60",
];

/// 歷史表現統計的回看天數（日曆天）。
const OUTCOME_LOOKBACK_DAYS: u64 = 364;

/// 歷史表現樣本少於此數時不顯示（避免少數樣本的勝率誤導）。
pub const MIN_OUTCOME_SAMPLES: i64 = 30;

/// 比較 RS 轉折要讀的交易日數（當天與前一個交易日）。
const RS_DAYS: u16 = 2;

/// 持股當天的相對強度轉折。
#[derive(Debug, Clone, PartialEq)]
struct RsTurn {
    stock_symbol: String,
    /// 前一個交易日的分數。
    previous: u8,
    /// 當天的分數。
    current: u8,
    /// 當天在全體中的名次。
    rank: i64,
    /// 當天計分的股票總數。
    universe: i64,
    direction: CrossDirection,
    /// 穿越的門檻，高門檻在前。
    thresholds: Vec<u8>,
}

/// 排程入口：讀今天持股的技術訊號並發送通知。
///
/// # Errors
///
/// 查詢持股或訊號失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let holdings = PgPortfolioRepository::new()
        .fetch_active_holdings(None)
        .await
        .context("fetch active holdings for technical alert failed")?;
    let symbols: BTreeSet<String> = holdings
        .into_iter()
        .map(|holding| holding.security_code)
        .collect();
    let date = Local::now().date_naive();
    let message = run(&PgQuoteRepository::new(), date, &symbols).await?;
    if let Some(message) = message {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 讀取持股訊號與 RS 轉折並組出通知內容；兩者都沒有時回 `None`。
async fn run(
    repo: &(impl TechnicalSignalQuery + RelativeStrengthQuery),
    date: NaiveDate,
    symbols: &BTreeSet<String>,
) -> Result<Option<String>> {
    let symbols: Vec<String> = symbols.iter().cloned().collect();
    let signals = repo
        .fetch_signals(date, &symbols)
        .await
        .context("fetch technical signals of holdings failed")?;
    let notable = notable(signals);
    let turns = match repo.fetch_relative_strength(date, RS_DAYS, &symbols).await {
        Ok(days) => rs_turns(&days, date),
        Err(why) => {
            tracing::warn!("持股技術訊號通知：讀取相對強度失敗，不列 RS 轉折: {why:#}");
            Vec::new()
        }
    };
    tracing::info!(
        "持股技術訊號通知: holdings={} signals={} rs_turns={}",
        symbols.len(),
        notable.len(),
        turns.len()
    );
    if notable.is_empty() && turns.is_empty() {
        return Ok(None);
    }
    let outcomes = if notable.is_empty() {
        Vec::new()
    } else {
        let from = date - Days::new(OUTCOME_LOOKBACK_DAYS);
        repo.fetch_signal_outcomes(from, date, &NOTIFY_INDICATORS)
            .await
            .unwrap_or_else(|why| {
                tracing::warn!("持股技術訊號通知：讀取訊號歷史表現失敗，不附勝率: {why:#}");
                Vec::new()
            })
    };
    Ok(build_message(
        date,
        &notable,
        &outcomes,
        &turns,
        holdings::stock_name,
    ))
}

/// 由依代號、再由新到舊排列的分數找出當天穿越門檻的持股；最新一筆不是 `date`（當天報酬
/// 還沒回填）或沒有前一個交易日的分數就略過。
fn rs_turns(days: &[RelativeStrengthDay], date: NaiveDate) -> Vec<RsTurn> {
    days.chunk_by(|a, b| a.stock_symbol == b.stock_symbol)
        .filter_map(|group| {
            let [latest, previous, ..] = group else {
                return None;
            };
            if latest.date != date {
                return None;
            }
            let (current, before) = (latest.rs_score, previous.rs_score);
            let crossed = crossings(before, current);
            let direction = crossed.first()?.1;
            Some(RsTurn {
                stock_symbol: latest.stock_symbol.clone(),
                previous: before,
                current,
                rank: latest.rank,
                universe: latest.universe,
                direction,
                thresholds: crossed
                    .into_iter()
                    .map(|(threshold, _)| threshold)
                    .collect(),
            })
        })
        .collect()
}

/// RS 轉折的說明，例如「相對強度 80 → 79，跌破 80 分（全體第 446 名／2228 檔）」。
fn turn_text(turn: &RsTurn) -> String {
    let verb = match turn.direction {
        CrossDirection::Up => "突破",
        CrossDirection::Down => "跌破",
    };
    let thresholds: Vec<String> = turn.thresholds.iter().map(u8::to_string).collect();
    format!(
        "相對強度 {} → {}，{verb} {} 分（全體第 {} 名／{} 檔）",
        turn.previous,
        turn.current,
        thresholds.join("、"),
        turn.rank,
        turn.universe
    )
}

/// 只留下 [`NOTIFY_INDICATORS`] 的訊號，依代號、再依來源的通知順序排列。
fn notable(mut signals: Vec<StockTechnicalSignal>) -> Vec<StockTechnicalSignal> {
    signals.retain(|signal| NOTIFY_INDICATORS.contains(&signal.signal.indicator));
    signals.sort_by_key(|signal| {
        (
            signal.stock_symbol.clone(),
            NOTIFY_INDICATORS
                .iter()
                .position(|indicator| *indicator == signal.signal.indicator),
        )
    });
    signals
}

/// 依股票（代號排序）分段列出訊號說明（附同一訊號的歷史表現）與 RS 轉折；兩者都沒有時回 `None`。
fn build_message(
    date: NaiveDate,
    signals: &[StockTechnicalSignal],
    outcomes: &[SignalOutcome],
    turns: &[RsTurn],
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    if signals.is_empty() && turns.is_empty() {
        return None;
    }
    let stocks: BTreeSet<&str> = signals
        .iter()
        .map(|signal| signal.stock_symbol.as_str())
        .chain(turns.iter().map(|turn| turn.stock_symbol.as_str()))
        .collect();
    let mut message = String::with_capacity(128 + (signals.len() + turns.len()) * 96);
    let _ = writeln!(
        &mut message,
        "持股技術訊號︰{}",
        text::escape_markdown_v2(date.to_string())
    );
    let mut with_outcome = false;
    for symbol in stocks {
        let _ = write!(
            &mut message,
            "\n{}\n",
            text::escape_markdown_v2(format!("{symbol} {}", stock_name(symbol)))
        );
        for signal in signals
            .iter()
            .filter(|signal| signal.stock_symbol == symbol)
        {
            let (line, attached) = signal_line(signal, outcomes);
            with_outcome |= attached;
            let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
        }
        for turn in turns.iter().filter(|turn| turn.stock_symbol == symbol) {
            let _ = writeln!(
                &mut message,
                "{}",
                text::escape_markdown_v2(format!("• {}", turn_text(turn)))
            );
        }
    }
    if with_outcome {
        let _ = write!(
            &mut message,
            "\n{}\n",
            text::escape_markdown_v2(format!(
                "＊括號內為近一年全市場同一訊號出現後 {OUTCOME_HORIZON} 個交易日的還原報酬，過去表現不代表未來結果"
            ))
        );
    }
    Some(message)
}

/// 訊號的一行說明，樣本足夠時附上歷史表現；第二個值表示是否附了歷史表現。
fn signal_line(signal: &StockTechnicalSignal, outcomes: &[SignalOutcome]) -> (String, bool) {
    let outcome = outcomes.iter().find(|outcome| {
        outcome.indicator == signal.signal.indicator
            && outcome.kind == signal.signal.kind.as_str()
            && outcome.samples >= MIN_OUTCOME_SAMPLES
    });
    match outcome {
        Some(outcome) => (
            format!(
                "• {}（{}）",
                signal.signal.description,
                outcome_text(outcome)
            ),
            true,
        ),
        None => (format!("• {}", signal.signal.description), false),
    }
}

/// 歷史表現的簡短文字，例如「20 日勝率 58%、平均 +1.2%，1234 次」。
fn outcome_text(outcome: &SignalOutcome) -> String {
    format!(
        "{OUTCOME_HORIZON} 日勝率 {:.0}%、平均 {:+.1}%，{} 次",
        outcome.win_rate * 100.0,
        outcome.avg_return,
        outcome.samples
    )
}

#[cfg(test)]
mod tests {
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
        let only_rs = build_message(date(), &[], &[], &turns[1..], |_| "台積電".to_owned())
            .expect("有 RS 轉折");
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
        let message = build_message(date(), &signals, &outcomes, &[], |_| "台積電".to_owned())
            .expect("有訊號");
        assert!(
            message.contains(r"• 跌破年線（20 日勝率 45%、平均 \-1\.2%，1234 次）"),
            "{message}"
        );
        assert!(message.contains("• 跌破季線\n"), "{message}");
        assert!(message.contains("• 均線黃金交叉\n"), "{message}");
        assert!(message.ends_with("過去表現不代表未來結果\n"), "{message}");

        let plain = build_message(date(), &signals[1..], &outcomes, &[], |_| String::new())
            .expect("有訊號");
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
}
