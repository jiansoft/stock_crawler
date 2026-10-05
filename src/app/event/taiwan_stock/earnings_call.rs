//! # 持股法說會通知
//!
//! 持股公司開完法說會、BigGo 整理出摘要後，推送一則 Telegram：摘要、展望與 Q&A 重點，
//! 附上 BigGo 的完整內容連結。
//!
//! ## 流程（每天 20:30 排程）
//!
//! 1. 取目前持股（未售出）的股票代號，排除 ETF（`00` 開頭）與特別股等含英文字母的代號——
//!    它們沒有法說會。
//! 2. 逐檔向 BigGo 取法說會清單，只看 [`LOOKBACK_DAYS`] 天內舉行的場次。
//! 3. 已通知過的場次（Redis `biggo:earnings_call:notified:{call_id}`）略過；AI 摘要還沒產生的
//!    場次這次不通知也不記錄，之後的排程再看，直到超過回看天數。
//! 4. 每場一則訊息。
//!
//! BigGo 的整理內容是其著作：只做個人通知，不寫進資料庫，也不經公開網站或 API 轉發。

use std::collections::BTreeSet;
use std::fmt::Write;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

use crate::{
    core::{alert, util::text},
    domain::portfolio::repository::PortfolioRepository,
    infra::{
        cache::SHARE,
        crawler::biggo::earnings_call::{self, EarningsCall, EarningsCallDetail},
        database::repository::portfolio::PgPortfolioRepository,
        nosql::redis::CLIENT,
    },
};

/// 只通知這麼多天內舉行的法說會；BigGo 通常當天或隔天就整理出摘要。
pub const LOOKBACK_DAYS: i64 = 7;

/// 已通知紀錄的保存時間（60 天，遠長於回看天數）。
const NOTIFIED_TTL_SECONDS: usize = 60 * 60 * 24 * 60;

/// 每個段落最多保留的字數；完整內容看 BigGo 連結。
const SECTION_MAX_CHARS: usize = 1_200;

/// 股票之間的請求間隔。
const STOCK_INTERVAL: Duration = Duration::from_millis(800);

/// 一輪通知的結果。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunSummary {
    /// 檢查的持股檔數。
    pub holdings: usize,
    /// 回看期間內找到的場次數。
    pub calls: usize,
    /// 這次送出通知的場次數。
    pub notified: usize,
    /// AI 摘要尚未產生、留待下次的場次數。
    pub pending: usize,
    /// 抓取失敗的次數（清單或內容）。
    pub failed: usize,
}

/// 單場法說會的處理結果。
enum Outcome {
    AlreadyNotified,
    Notified,
    Pending,
    Failed,
}

/// 排程入口：通知持股近期法說會的 AI 整理內容。
///
/// 單檔失敗只記錄、不中斷；只有查不到持股時回傳錯誤。
pub async fn execute() -> Result<()> {
    let holdings = PgPortfolioRepository::new()
        .fetch_active_holdings(None)
        .await
        .context("fetch active holdings for earnings call notification failed")?;
    let symbols: BTreeSet<String> = holdings
        .into_iter()
        .map(|holding| holding.security_code)
        .filter(|symbol| has_earnings_calls(symbol))
        .collect();

    let today = Local::now().date_naive();
    let mut summary = RunSummary {
        holdings: symbols.len(),
        ..Default::default()
    };
    for symbol in &symbols {
        match earnings_call::list(symbol).await {
            Ok(calls) => {
                for call in recent_calls(&calls, today) {
                    summary.calls += 1;
                    match notify(symbol, call).await {
                        Outcome::Notified => summary.notified += 1,
                        Outcome::Pending => summary.pending += 1,
                        Outcome::Failed => summary.failed += 1,
                        Outcome::AlreadyNotified => {}
                    }
                }
            }
            Err(why) => {
                summary.failed += 1;
                tracing::warn!("BigGo 法說會清單抓取失敗: stock_symbol={symbol}, error={why:#}");
            }
        }
        tokio::time::sleep(STOCK_INTERVAL).await;
    }

    tracing::info!(
        "持股法說會通知結束: holdings={}, calls={}, notified={}, pending={}, failed={}",
        summary.holdings,
        summary.calls,
        summary.notified,
        summary.pending,
        summary.failed
    );
    Ok(())
}

/// 處理單場法說會：未通知過且摘要已產生才送出，送出後記錄。
async fn notify(stock_symbol: &str, call: &EarningsCall) -> Outcome {
    let key = notified_key(&call.call_id);
    match CLIENT.get_bool(&key).await {
        Ok(true) => return Outcome::AlreadyNotified,
        Ok(false) => {}
        // Redis 異常時當作尚未通知：寧可重複發一次，也不要因為快取故障而漏發。
        Err(why) => tracing::warn!("讀取法說會通知紀錄 {key} 失敗，視為尚未通知: {why:?}"),
    }

    let detail = match earnings_call::detail(stock_symbol, &call.call_id).await {
        Ok(detail) => detail,
        Err(why) => {
            tracing::warn!(
                "BigGo 法說會內容抓取失敗: stock_symbol={stock_symbol}, call_id={}, error={why:#}",
                call.call_id
            );
            return Outcome::Failed;
        }
    };
    if !detail.is_ready() {
        return Outcome::Pending;
    }

    let name = stock_name(stock_symbol);
    alert::send_message(&build_message(stock_symbol, &name, call, &detail)).await;
    if let Err(why) = CLIENT.set(&key, true, NOTIFIED_TTL_SECONDS).await {
        tracing::warn!("寫入法說會通知紀錄 {key} 失敗，下次排程可能重複通知: {why:?}");
    }
    Outcome::Notified
}

/// 是否可能有法說會：排除 ETF／ETN（`00` 開頭）與特別股、受益證券等含英文字母的代號。
fn has_earnings_calls(stock_symbol: &str) -> bool {
    !stock_symbol.starts_with("00") && stock_symbol.chars().all(|c| c.is_ascii_digit())
}

/// 回看期間內（含今天）舉行的場次。
fn recent_calls(calls: &[EarningsCall], today: NaiveDate) -> Vec<&EarningsCall> {
    let since = today - chrono::Duration::days(LOOKBACK_DAYS);
    calls
        .iter()
        .filter(|call| call.date >= since && call.date <= today)
        .collect()
}

/// 已通知紀錄的 Redis key。
fn notified_key(call_id: &str) -> String {
    format!("biggo:earnings_call:notified:{call_id}")
}

/// 從股票主檔快取取股名；查不到時回空字串。
fn stock_name(stock_symbol: &str) -> String {
    SHARE
        .stocks
        .read()
        .ok()
        .and_then(|stocks| {
            stocks
                .get(stock_symbol)
                .map(|stock| stock.name().to_string())
        })
        .unwrap_or_default()
}

/// 組出通知訊息（Telegram MarkdownV2）。
fn build_message(
    stock_symbol: &str,
    stock_name: &str,
    call: &EarningsCall,
    detail: &EarningsCallDetail,
) -> String {
    let mut message = String::with_capacity(4096);
    let _ = writeln!(
        &mut message,
        "持股法說會︰{} {}（{}）",
        text::escape_markdown_v2(stock_symbol),
        text::escape_markdown_v2(stock_name),
        text::escape_markdown_v2(call.date.to_string())
    );
    let title = if detail.title.is_empty() {
        &call.title
    } else {
        &detail.title
    };
    let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(title.as_str()));
    for (heading, body) in [
        ("摘要", &detail.summary),
        ("展望", &detail.outlook),
        ("Q&A 重點", &detail.qa_highlights),
    ] {
        let body = clean_section(body);
        if body.is_empty() {
            continue;
        }
        let _ = write!(
            &mut message,
            "\n【{}】\n{}\n",
            text::escape_markdown_v2(heading),
            text::escape_markdown_v2(body)
        );
    }
    let _ = write!(
        &mut message,
        "\n完整內容︰[BigGo]({})",
        text::escape_markdown_v2(earnings_call::page_url(stock_symbol, &call.call_id))
    );
    message
}

/// 整理段落文字：去掉 Markdown 粗體標記（Telegram 不支援 `**`）、多餘空白，
/// 超過 [`SECTION_MAX_CHARS`] 字時截斷並加上「…」。
fn clean_section(body: &str) -> String {
    let cleaned = body.replace("**", "");
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= SECTION_MAX_CHARS {
        return cleaned.to_string();
    }
    let mut truncated: String = cleaned.chars().take(SECTION_MAX_CHARS).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    fn call(call_id: &str, date: NaiveDate) -> EarningsCall {
        EarningsCall {
            call_id: call_id.to_string(),
            date,
            title: "台積電 2026-10-05 法說會".to_string(),
            has_transcript: true,
        }
    }

    #[test]
    fn has_earnings_calls_skips_etfs_and_preferred_shares() {
        assert!(has_earnings_calls("2330"));
        assert!(has_earnings_calls("6505"));
        for symbol in ["0050", "00878", "2887G", "2887Z1"] {
            assert!(!has_earnings_calls(symbol), "{symbol}");
        }
    }

    /// 只看回看天數內（含今天）的場次，未來日期與太舊的都不看。
    #[test]
    fn recent_calls_keeps_the_lookback_window() {
        let calls = vec![
            call("future", day(10, 6)),
            call("today", day(10, 5)),
            call("edge", day(9, 28)),
            call("old", day(9, 27)),
        ];
        let ids: Vec<&str> = recent_calls(&calls, day(10, 5))
            .into_iter()
            .map(|call| call.call_id.as_str())
            .collect();
        assert_eq!(ids, vec!["today", "edge"]);
    }

    #[test]
    fn clean_section_strips_bold_markers_and_truncates() {
        assert_eq!(
            clean_section("  **Q: 展望**\nA: 樂觀  "),
            "Q: 展望\nA: 樂觀"
        );
        let long = "字".repeat(SECTION_MAX_CHARS + 10);
        let cleaned = clean_section(&long);
        assert_eq!(cleaned.chars().count(), SECTION_MAX_CHARS + 1);
        assert!(cleaned.ends_with('…'));
    }

    /// 訊息包含標頭、三個段落與 BigGo 連結，特殊字元依 MarkdownV2 跳脫，空段落略過。
    #[test]
    fn build_message_formats_sections_and_link() {
        let detail = EarningsCallDetail {
            call_id: "TW_2330.TW_2026-07-16".to_string(),
            date: Some(day(7, 16)),
            title: "台積電 2026-07-16 法說會".to_string(),
            summary: "營收 402 億美元（季增 11.3%）".to_string(),
            operations: "不會出現".to_string(),
            outlook: String::new(),
            qa_highlights: "**Q: 資本支出?**\nA: 上修。".to_string(),
        };
        let message = build_message(
            "2330",
            "台積電",
            &call("TW_2330.TW_2026-07-16", day(7, 16)),
            &detail,
        );
        assert!(message.starts_with("持股法說會︰2330 台積電（2026\\-07\\-16）\n"));
        assert!(message.contains("【摘要】\n營收 402 億美元（季增 11\\.3%）"));
        assert!(!message.contains("【展望】"), "空段落不輸出");
        assert!(!message.contains("不會出現"), "營運重點不放進通知");
        assert!(message.contains("【Q&A 重點】\nQ: 資本支出?\nA: 上修。"));
        assert!(message.ends_with(
            "完整內容︰[BigGo](https://finance\\.biggo\\.com\\.tw/quote/2330\\.TW/earnings\\-call/TW\\_2330\\.TW\\_2026\\-07\\-16)"
        ));
    }

    #[test]
    fn notified_key_uses_the_call_id() {
        assert_eq!(
            notified_key("TW_2330.TW_2026-07-16"),
            "biggo:earnings_call:notified:TW_2330.TW_2026-07-16"
        );
    }
}
