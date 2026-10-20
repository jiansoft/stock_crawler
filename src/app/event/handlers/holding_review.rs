//! 持股定期回顧通知：月營收公布、季報公布、外資持股明顯變化時各發一則彙總訊息。
//!
//! 三個事件的處理流程一樣——收到「某期資料已更新」後自己去查持股、組訊息、送出——
//! 因此放在同一個模組共用排版工具；月營收與季報訊息的組裝與格式化在子模組 `message`。
//!
//! 只看**目前持有**的股票。全市場每個月有上千檔公布營收，全推等於製造雜訊；
//! 這兩則通知的用途是「我手上的股票發生了什麼」，不是市場掃描。
//!
//! 月營收通知列出**每一檔**持股（不設年增率門檻）並依年增率由高到低排序，
//! 讓整個組合的動能在同一則訊息裡一次排開，便於決定加碼或減持。
//!
//! 月營收與季報排程每天都會重抓整期資料並派發事件，因此以 Redis 記錄每一期已通知過的
//! 持股代號：只有出現尚未通知過的持股（新公布或新買進）才送出，並在補發時標出新增的那幾檔。

use std::collections::BTreeSet;
use std::fmt::Write;

use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::app::event::taiwan_stock::format_decimal_with_commas;
// 通知走 core::alert（port），跳脫工具走 core::util::text——
// app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向。
use crate::core::{alert, util::text};
use crate::domain::financial::entity::{
    EpsEstimateBasis, HoldingFinancialAlert, HoldingRevenueAlert, MarginTrend,
};
use crate::domain::financial::repository::FinancialRepository;
use crate::domain::foreign_holding::entity::{
    ForeignHoldingDirection, HoldingForeignHoldingAlert, SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS,
};
use crate::domain::foreign_holding::repository::ForeignHoldingRepository;
use crate::domain::portfolio::health::REVENUE_DECLINE_WARNING_MONTHS;
use crate::infra::database::repository::financial::PgFinancialRepository;
use crate::infra::database::repository::foreign_holding::PgForeignHoldingRepository;
use crate::infra::nosql::redis::{CLIENT, RedisError};

use super::EventDispatcher;

mod message;

/// 月營收已通知代號的保存時間。營收排程只回補上個月，90 天足以涵蓋整個公布期。
const REVENUE_NOTIFIED_TTL_SECONDS: usize = 60 * 60 * 24 * 90;

/// 季報已通知代號的保存時間。季報回補視窗會跨越申報截止日前後數個月，保留 180 天。
const FINANCIAL_NOTIFIED_TTL_SECONDS: usize = 60 * 60 * 24 * 180;

/// 外資持股變化的去重期間：同一檔、同一方向 20 天內只通知一次，與 20 日變化的觀察窗一致。
/// 20 日變化會連續多天都超過門檻，不去重就會天天重複推播同一件事。
const FOREIGN_HOLDING_NOTIFIED_TTL_SECONDS: usize = 60 * 60 * 24 * 20;

/// 連續增減持天數達到這個值才在訊息裡註明，一兩天的方向沒有意義。
const FOREIGN_HOLDING_STREAK_NOTE_DAYS: i32 = 3;

/// 補發通知時標在新增持股前的記號。
const NEW_HOLDING_MARK: &str = "🆕 ";

impl EventDispatcher {
    /// 處理 `MonthlyRevenueUpdated` 事件：推播全部持股的月營收。
    pub(super) async fn handle_monthly_revenue_updated(date: i64) -> Result<()> {
        let repo = PgFinancialRepository::new();
        let alerts = repo.fetch_holding_revenue_alerts(date).await?;

        if alerts.is_empty() {
            tracing::info!("持股月營收通知：{} 沒有任何持股公布月營收", date);
            return Ok(());
        }

        let sent = Self::send_if_new_holdings(
            &format!("holding_review:revenue:{date}"),
            REVENUE_NOTIFIED_TTL_SECONDS,
            alerts.iter().map(|alert| alert.stock_symbol.as_str()),
            |new_symbols| Self::build_revenue_message(date, &alerts, new_symbols),
        )
        .await;
        if !sent {
            tracing::info!("持股月營收通知：{} 沒有新公布的持股，略過重複通知", date);
        }

        Ok(())
    }

    /// 處理 `QuarterlyFinancialsUpdated` 事件：推播持股的最新一季財報。
    pub(super) async fn handle_quarterly_financials_updated(
        year: i32,
        quarter: &str,
    ) -> Result<()> {
        let repo = PgFinancialRepository::new();
        let alerts = repo.fetch_holding_financial_alerts(year, quarter).await?;

        if alerts.is_empty() {
            tracing::info!("持股財報通知：{} {} 沒有持股的財報", year, quarter);
            return Ok(());
        }

        let sent = Self::send_if_new_holdings(
            &format!("holding_review:financial:{year}{quarter}"),
            FINANCIAL_NOTIFIED_TTL_SECONDS,
            alerts.iter().map(|alert| alert.stock_symbol.as_str()),
            |new_symbols| Self::build_financial_message(year, quarter, &alerts, new_symbols),
        )
        .await;
        if !sent {
            tracing::info!(
                "持股財報通知：{} {} 沒有新公布的持股，略過重複通知",
                year,
                quarter
            );
        }

        Ok(())
    }

    /// 處理 `ForeignHoldingsUpdated` 事件：持股中近 20 日外資持股比率變化達門檻者彙總推播。
    ///
    /// 每檔、每個方向各用一個會過期的 Redis 鍵去重（20 天），而不是像月營收那樣共用一個集合：
    /// 共用集合每次寫入都會延長 TTL，已通知過的股票就永遠不會再被提醒。
    pub(super) async fn handle_foreign_holdings_updated(date: NaiveDate) -> Result<()> {
        let repo = PgForeignHoldingRepository::new();
        let alerts = repo.fetch_holding_alerts().await?;

        let mut increases = Vec::new();
        let mut decreases = Vec::new();
        let mut keys = Vec::new();
        for alert in &alerts {
            let Some(direction) =
                alert.significant_direction(SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS)
            else {
                continue;
            };
            let key = Self::foreign_holding_key(direction, &alert.stock_symbol);
            if Self::is_already_notified(&key).await {
                continue;
            }
            match direction {
                ForeignHoldingDirection::Increase => increases.push(alert),
                ForeignHoldingDirection::Decrease => decreases.push(alert),
            }
            keys.push(key);
        }

        if keys.is_empty() {
            tracing::info!("持股外資持股通知：{} 沒有新達門檻的持股", date);
            return Ok(());
        }

        alert::send_message(&Self::build_foreign_holding_message(
            date, &increases, &decreases,
        ))
        .await;

        for key in keys {
            if let Err(why) = CLIENT
                .set(&key, date.to_string(), FOREIGN_HOLDING_NOTIFIED_TTL_SECONDS)
                .await
            {
                tracing::warn!("寫入外資持股通知紀錄 {} 失敗，可能重複通知: {:?}", key, why);
            }
        }

        Ok(())
    }

    /// 外資持股通知的 Redis 去重鍵。
    fn foreign_holding_key(direction: ForeignHoldingDirection, stock_symbol: &str) -> String {
        format!("holding_review:qfii:{}:{}", direction.code(), stock_symbol)
    }

    /// 某個去重鍵是否仍在有效期內；Redis 異常時視為尚未通知（寧可重複也不要漏發）。
    async fn is_already_notified(key: &str) -> bool {
        match CLIENT.get_bytes(key).await {
            Ok(_) => true,
            Err(RedisError::NotFound) => false,
            Err(why) => {
                tracing::warn!("讀取外資持股通知紀錄 {} 失敗，視為尚未通知: {:?}", key, why);
                false
            }
        }
    }

    /// 組出外資持股變化通知：增持在前、減持在後，各自依 20 日變化幅度排序。
    fn build_foreign_holding_message(
        date: NaiveDate,
        increases: &[&HoldingForeignHoldingAlert],
        decreases: &[&HoldingForeignHoldingAlert],
    ) -> String {
        let mut msg = String::with_capacity((increases.len() + decreases.len()) * 160 + 128);
        let _ = writeln!(
            &mut msg,
            "{} 持股外資持股明顯變化（近 20 個交易日 ±{} 個百分點以上）︰",
            text::escape_markdown_v2(date.to_string()),
            text::escape_markdown_v2(format_decimal_with_commas(
                SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS
            ))
        );

        let mut sorted_increases = increases.to_vec();
        sorted_increases.sort_by_key(|alert| std::cmp::Reverse(alert.change_20d));
        let mut sorted_decreases = decreases.to_vec();
        sorted_decreases.sort_by_key(|alert| alert.change_20d);

        for (title, group) in [
            ("外資增持", sorted_increases),
            ("外資減持", sorted_decreases),
        ] {
            if group.is_empty() {
                continue;
            }
            let _ = writeln!(&mut msg, "  {title}︰");
            for alert in group {
                let _ = writeln!(
                    &mut msg,
                    "    {} {} 持股︰{}% 20日︰{} 5日︰{}{}",
                    Self::stock_link(&alert.stock_symbol),
                    text::escape_markdown_v2(&alert.stock_name),
                    text::escape_markdown_v2(format_decimal_with_commas(
                        alert.share_holding_percentage
                    )),
                    text::escape_markdown_v2(Self::format_optional_change(alert.change_20d)),
                    text::escape_markdown_v2(Self::format_optional_change(alert.change_5d)),
                    text::escape_markdown_v2(Self::format_streak(alert.streak))
                );
            }
        }

        msg
    }

    /// 百分點變化帶正負號；歷史不足而沒有值時顯示「—」，不畫成 0。
    fn format_optional_change(change: Option<Decimal>) -> String {
        change.map_or_else(|| "—".to_string(), Self::with_sign)
    }

    /// 連續增減持達 [`FOREIGN_HOLDING_STREAK_NOTE_DAYS`] 天以上才註明，其餘回傳空字串。
    fn format_streak(streak: i32) -> String {
        if streak >= FOREIGN_HOLDING_STREAK_NOTE_DAYS {
            format!(" 連續增持 {streak} 日")
        } else if streak <= -FOREIGN_HOLDING_STREAK_NOTE_DAYS {
            format!(" 連續減持 {} 日", -streak)
        } else {
            String::new()
        }
    }

    /// 只在出現尚未通知過的持股時送出訊息，並把已通知的代號寫回 Redis；回傳是否有送出。
    ///
    /// 第一次通知時全部都是新的，`build_message` 收到 `None`，不逐檔標記；
    /// 之後的補發才傳入新增代號，讓訊息標出這次多了哪幾檔。
    /// 已通知集合只增不減，賣出後再買回的持股不會在同一期被重複通知。
    async fn send_if_new_holdings<'a>(
        key: &str,
        ttl_in_seconds: usize,
        symbols: impl IntoIterator<Item = &'a str>,
        build_message: impl FnOnce(Option<&BTreeSet<String>>) -> String,
    ) -> bool {
        let mut notified = Self::load_notified_symbols(key).await;
        let new_symbols = Self::find_new_symbols(&notified, symbols);
        if new_symbols.is_empty() {
            return false;
        }

        let highlight = (!notified.is_empty()).then_some(&new_symbols);
        alert::send_message(&build_message(highlight)).await;

        notified.extend(new_symbols);
        let value = notified.into_iter().collect::<Vec<_>>().join(",");
        if let Err(why) = CLIENT.set(key, value, ttl_in_seconds).await {
            tracing::warn!(
                "寫入持股通知紀錄 {} 失敗，下次排程可能重複通知: {:?}",
                key,
                why
            );
        }

        true
    }

    /// 讀取某一期已通知過的持股代號。
    ///
    /// Redis 異常時當作尚未通知：寧可重複發一次，也不要因為快取故障而漏發。
    async fn load_notified_symbols(key: &str) -> BTreeSet<String> {
        match CLIENT.get_bytes(key).await {
            Ok(bytes) => Self::parse_notified_symbols(&String::from_utf8_lossy(&bytes)),
            Err(RedisError::NotFound) => BTreeSet::new(),
            Err(why) => {
                tracing::warn!("讀取持股通知紀錄 {} 失敗，視為尚未通知: {:?}", key, why);
                BTreeSet::new()
            }
        }
    }

    /// 解析以逗號分隔的代號清單，忽略空白與空項目。
    fn parse_notified_symbols(value: &str) -> BTreeSet<String> {
        value
            .split(',')
            .map(str::trim)
            .filter(|symbol| !symbol.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// 找出這次查到、但還沒通知過的持股代號。
    fn find_new_symbols<'a>(
        notified: &BTreeSet<String>,
        symbols: impl IntoIterator<Item = &'a str>,
    ) -> BTreeSet<String> {
        symbols
            .into_iter()
            .filter(|symbol| !notified.contains(*symbol))
            .map(str::to_string)
            .collect()
    }
}

#[cfg(test)]
mod tests;
