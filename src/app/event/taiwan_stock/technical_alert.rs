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
//! - 排程在 19:15（週一至週五），排在 19:00 收盤資料完整性檢查之後，重跑收盤匯總的訊號也已定案。
//! - 沒有任何持股出現訊號（含休市日沒有訊號）時不發送。

use std::collections::BTreeSet;
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate};

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::{
        portfolio::repository::PortfolioRepository,
        quote::technical::{
            FIFTY_TWO_WEEK_INDICATOR, OUTCOME_HORIZON, SignalOutcome, StockTechnicalSignal,
            TechnicalSignalQuery,
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

/// 讀取持股訊號並組出通知內容；沒有要通知的訊號時回 `None`。
async fn run(
    repo: &impl TechnicalSignalQuery,
    date: NaiveDate,
    symbols: &BTreeSet<String>,
) -> Result<Option<String>> {
    let symbols: Vec<String> = symbols.iter().cloned().collect();
    let signals = repo
        .fetch_signals(date, &symbols)
        .await
        .context("fetch technical signals of holdings failed")?;
    let notable = notable(signals);
    tracing::info!(
        "持股技術訊號通知: holdings={} signals={}",
        symbols.len(),
        notable.len()
    );
    if notable.is_empty() {
        return Ok(None);
    }
    let from = date - Days::new(OUTCOME_LOOKBACK_DAYS);
    let outcomes = repo
        .fetch_signal_outcomes(from, date, &NOTIFY_INDICATORS)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("持股技術訊號通知：讀取訊號歷史表現失敗，不附勝率: {why:#}");
            Vec::new()
        });
    Ok(build_message(
        date,
        &notable,
        &outcomes,
        holdings::stock_name,
    ))
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

/// 依股票分段列出訊號說明，附上同一訊號的歷史表現；沒有訊號時回 `None`。
fn build_message(
    date: NaiveDate,
    signals: &[StockTechnicalSignal],
    outcomes: &[SignalOutcome],
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    if signals.is_empty() {
        return None;
    }
    let mut message = String::with_capacity(128 + signals.len() * 96);
    let _ = writeln!(
        &mut message,
        "持股技術訊號︰{}",
        text::escape_markdown_v2(date.to_string())
    );
    let mut current: Option<&str> = None;
    let mut with_outcome = false;
    for signal in signals {
        if current != Some(signal.stock_symbol.as_str()) {
            current = Some(&signal.stock_symbol);
            let _ = write!(
                &mut message,
                "\n{}\n",
                text::escape_markdown_v2(format!(
                    "{} {}",
                    signal.stock_symbol,
                    stock_name(&signal.stock_symbol)
                ))
            );
        }
        let outcome = outcomes.iter().find(|outcome| {
            outcome.indicator == signal.signal.indicator
                && outcome.kind == signal.signal.kind.as_str()
                && outcome.samples >= MIN_OUTCOME_SAMPLES
        });
        let line = match outcome {
            Some(outcome) => {
                with_outcome = true;
                format!(
                    "• {}（{}）",
                    signal.signal.description,
                    outcome_text(outcome)
                )
            }
            None => format!("• {}", signal.signal.description),
        };
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
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

    /// 回傳固定訊號與歷史表現、記錄查詢條件的假倉儲；`fail`／`outcomes_fail` 時回錯誤。
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

        let plain =
            build_message(date(), &signals[1..], &outcomes, |_| String::new()).expect("有訊號");
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
}
