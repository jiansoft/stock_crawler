//! # 持股重大訊息通知
//!
//! 把持股公司在公開資訊觀測站發布的重大訊息（減資、私募、處分資產、更名、澄清媒體報導…）
//! 推送到 Telegram。
//!
//! ## 流程（每天 08:30、18:30 排程）
//!
//! 1. 取目前持股的普通股代號（[`holdings::common_stock_holdings`]）。
//! 2. 抓上市、上櫃最近一天的重大訊息開放資料；一邊失敗不影響另一邊。
//!    上櫃那份會比上市晚一天，所以一天跑兩次。
//! 3. 以「代號＋主旨」去重（Redis `material_news:{代號}:{主旨雜湊}`，保存 [`NOTIFIED_TTL_DAYS`] 天）：
//!    更名、面額變更等公告在三個月的公告期間內每天重發，同一主旨只通知一次。
//! 4. 有新訊息才發一則通知，依發言時間排列；說明超過 [`DESCRIPTION_MAX_CHARS`] 字截斷。

use std::fmt::Write;

use anyhow::{Context, Result};

use super::holdings;
use crate::{
    core::{alert, util::text},
    infra::{
        crawler::mops::material_news::{self, MaterialNews},
        nosql::redis::CLIENT,
    },
};

/// 已通知紀錄的保存天數，涵蓋三個月的公告期間。
const NOTIFIED_TTL_DAYS: usize = 120;

/// 說明最多保留的字數；完整內容請到公開資訊觀測站查詢。
const DESCRIPTION_MAX_CHARS: usize = 400;

/// 排程入口：通知持股公司的新重大訊息。
///
/// 來源或單則處理失敗只記錄；只有查不到持股時回傳錯誤。
pub async fn execute() -> Result<()> {
    let symbols = holdings::common_stock_holdings()
        .await
        .context("fetch active holdings for material news notification failed")?;

    let mut news = Vec::new();
    for (market, result) in [
        ("上市", material_news::visit_listed().await),
        ("上櫃", material_news::visit_otc().await),
    ] {
        match result {
            Ok(mut rows) => news.append(&mut rows),
            Err(why) => tracing::warn!("{market}重大訊息抓取失敗: {why:#}"),
        }
    }

    let mut fresh = Vec::new();
    for item in news
        .into_iter()
        .filter(|item| symbols.contains(&item.stock_symbol))
    {
        let key = notified_key(&item);
        // NX 寫入成功才是第一次看到；Redis 失敗時寧可重複通知也不要漏。
        match CLIENT
            .set_if_absent(&key, true, NOTIFIED_TTL_DAYS * 24 * 60 * 60)
            .await
        {
            Ok(true) => fresh.push(item),
            Ok(false) => {}
            Err(why) => {
                tracing::warn!("寫入重大訊息通知紀錄 {key} 失敗，視為新訊息: {why:?}");
                fresh.push(item);
            }
        }
    }

    if !fresh.is_empty() {
        fresh.sort_by_key(|item| item.spoken_at);
        alert::send_message(&build_message(&fresh, holdings::stock_name)).await;
    }
    tracing::info!(
        "持股重大訊息通知結束: holdings={}, notified={}",
        symbols.len(),
        fresh.len()
    );
    Ok(())
}

/// 去重 key：同一家公司同一主旨只通知一次。
fn notified_key(item: &MaterialNews) -> String {
    format!(
        "material_news:{}:{:016x}",
        item.stock_symbol,
        fnv1a(&item.subject)
    )
}

/// FNV-1a 64 位元雜湊；std 的 `DefaultHasher` 不保證跨版本穩定，存進 Redis 的 key 不能用它。
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// 組出通知訊息（Telegram MarkdownV2）。
///
/// `stock_name` 由呼叫端提供（正式流程讀股票主檔快取），查不到時改用公告裡的公司名稱。
fn build_message(items: &[MaterialNews], stock_name: impl Fn(&str) -> String) -> String {
    let mut message = String::with_capacity(512 * items.len());
    let _ = writeln!(&mut message, "持股重大訊息");
    for item in items {
        let name = Some(stock_name(&item.stock_symbol))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| item.company_name.clone());
        let heading = format!(
            "{} {}（{}{}）",
            item.stock_symbol,
            name,
            item.spoken_at.format("%Y-%m-%d %H:%M"),
            if item.clause.is_empty() {
                String::new()
            } else {
                format!("，{}", item.clause)
            }
        );
        let _ = write!(
            &mut message,
            "\n*{}*\n{}\n",
            text::escape_markdown_v2(heading),
            text::escape_markdown_v2(item.subject.as_str())
        );
        let description = truncate(&item.description);
        if !description.is_empty() {
            let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(description));
        }
    }
    message
}

/// 說明超過上限時截斷並加上「…」。
fn truncate(description: &str) -> String {
    if description.chars().count() <= DESCRIPTION_MAX_CHARS {
        return description.to_string();
    }
    let mut truncated: String = description.chars().take(DESCRIPTION_MAX_CHARS).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

    use super::*;

    fn item(symbol: &str, subject: &str, description: &str) -> MaterialNews {
        MaterialNews {
            stock_symbol: symbol.to_string(),
            company_name: "公告名稱".to_string(),
            spoken_at: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
                NaiveTime::from_hms_opt(7, 0, 3).unwrap(),
            ),
            subject: subject.to_string(),
            clause: "第51款".to_string(),
            description: description.to_string(),
        }
    }

    /// 雜湊固定、不因版本改變；同主旨同 key、不同主旨或不同公司不同 key。
    #[test]
    fn notified_key_is_stable_per_symbol_and_subject() {
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
        let first = notified_key(&item("2072", "公告更名", "第一天"));
        let repeated = notified_key(&item("2072", "公告更名", "第二天說明不同"));
        assert_eq!(first, repeated);
        assert_ne!(first, notified_key(&item("2072", "公告面額變更", "")));
        assert_ne!(first, notified_key(&item("6949", "公告更名", "")));
        assert!(first.starts_with("material_news:2072:"));
    }

    #[test]
    fn truncate_limits_long_descriptions() {
        assert_eq!(truncate("短說明"), "短說明");
        let long = "字".repeat(DESCRIPTION_MAX_CHARS + 5);
        let cut = truncate(&long);
        assert_eq!(cut.chars().count(), DESCRIPTION_MAX_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    /// 標題粗體含代號、股名、發言時間與條款；股名查不到時用公告的公司名稱。
    #[test]
    fn build_message_formats_each_item() {
        let items = vec![
            item(
                "2072",
                "公告本公司更名為「世紀能源」",
                "1.事實發生日：民國115年08月24日",
            ),
            item("6949", "公告面額變更", ""),
        ];
        let message = build_message(&items, |symbol| {
            if symbol == "2072" {
                "世紀風電".to_string()
            } else {
                String::new()
            }
        });
        assert!(message.starts_with("持股重大訊息\n"), "{message}");
        assert!(
            message.contains(
                "\n*2072 世紀風電（2026\\-10\\-05 07:00，第51款）*\n公告本公司更名為「世紀能源」\n1\\.事實發生日：民國115年08月24日\n"
            ),
            "{message}"
        );
        assert!(
            message.contains("\n*6949 公告名稱（2026\\-10\\-05 07:00，第51款）*\n公告面額變更\n"),
            "{message}"
        );
    }
}
