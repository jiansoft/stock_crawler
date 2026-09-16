//! 持股定期回顧通知：月營收公布與季報公布後各發一則彙總訊息。
//!
//! 兩個事件的處理流程一樣——收到「某期資料已更新」後自己去查持股、組訊息、送出——
//! 因此放在同一個模組共用排版工具。
//!
//! 只看**目前持有**的股票。全市場每個月有數百檔營收年增率超過門檻，全推等於製造雜訊；
//! 這兩則通知的用途是「我手上的股票發生了什麼」，不是市場掃描。

use std::fmt::Write;

use anyhow::Result;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::app::event::taiwan_stock::format_decimal_with_commas;
// 通知走 core::alert（port），跳脫工具走 core::util::text——
// app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向。
use crate::core::{alert, util::text};
use crate::domain::financial::entity::{HoldingFinancialAlert, HoldingRevenueAlert};
use crate::domain::financial::repository::FinancialRepository;
use crate::infra::database::repository::financial::PgFinancialRepository;

use super::EventDispatcher;

/// 月營收年增率的推播門檻（%，取絕對值）。
///
/// 大跌與大漲都值得看，所以比較的是絕對值。20% 是經驗值：低於這個幅度的月營收波動
/// 在台股非常普遍（尤其是工作天數差異造成的），推播出來會很快被當成雜訊而忽略。
const REVENUE_YOY_THRESHOLD: Decimal = dec!(20);

impl EventDispatcher {
    /// 處理 `MonthlyRevenueUpdated` 事件：推播持股中年增率超過門檻的月營收。
    pub(super) async fn handle_monthly_revenue_updated(date: i64) -> Result<()> {
        let repo = PgFinancialRepository::new();
        let alerts = repo
            .fetch_holding_revenue_alerts(date, REVENUE_YOY_THRESHOLD)
            .await?;

        if alerts.is_empty() {
            tracing::info!("持股月營收通知：{} 沒有超過門檻的項目", date);
            return Ok(());
        }

        alert::send_message(&Self::build_revenue_message(date, &alerts)).await;

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

        alert::send_message(&Self::build_financial_message(year, quarter, &alerts)).await;

        Ok(())
    }

    /// 組出月營收通知訊息。
    fn build_revenue_message(date: i64, alerts: &[HoldingRevenueAlert]) -> String {
        let mut msg = String::with_capacity(1024);
        let _ = writeln!(
            &mut msg,
            "{} 持股月營收（年增率達 {}%）︰",
            text::escape_markdown_v2(Self::format_revenue_month(date)),
            text::escape_markdown_v2(REVENUE_YOY_THRESHOLD.normalize().to_string())
        );

        for alert in alerts {
            let _ = writeln!(
                &mut msg,
                "    {} {} 營收︰{}元 年增︰{}% 月增︰{}%",
                Self::stock_link(&alert.stock_symbol),
                text::escape_markdown_v2(&alert.stock_name),
                text::escape_markdown_v2(format_decimal_with_commas(alert.monthly)),
                text::escape_markdown_v2(Self::with_sign(alert.compared_with_last_year_same_month)),
                text::escape_markdown_v2(Self::with_sign(alert.compared_with_last_month))
            );
        }

        msg
    }

    /// 組出季報通知訊息。
    fn build_financial_message(
        year: i32,
        quarter: &str,
        alerts: &[HoldingFinancialAlert],
    ) -> String {
        let mut msg = String::with_capacity(1024);
        let _ = writeln!(
            &mut msg,
            "{} {} 持股財報︰",
            text::escape_markdown_v2(year.to_string()),
            text::escape_markdown_v2(quarter)
        );

        for alert in alerts {
            let _ = writeln!(
                &mut msg,
                "    {} {} EPS︰{}元{} ROE︰{}% 毛利率︰{}%",
                Self::stock_link(&alert.stock_symbol),
                text::escape_markdown_v2(&alert.stock_name),
                text::escape_markdown_v2(format_decimal_with_commas(alert.earnings_per_share)),
                Self::format_eps_comparison(alert),
                text::escape_markdown_v2(format_decimal_with_commas(alert.return_on_equity)),
                text::escape_markdown_v2(format_decimal_with_commas(alert.gross_profit))
            );
        }

        msg
    }

    /// 組出「（去年同季 X，±Y）」這段比較文字；查不到去年同季時回傳空字串。
    ///
    /// 沒有可比對象時寧可不顯示，也不要把「查無資料」畫成 0 —— 那會看起來像獲利歸零。
    fn format_eps_comparison(alert: &HoldingFinancialAlert) -> String {
        let Some(last_year) = alert.last_year_earnings_per_share else {
            return String::new();
        };

        let diff = alert.earnings_per_share - last_year;
        text::escape_markdown_v2(format!(
            "（去年同季 {}，{}）",
            format_decimal_with_commas(last_year),
            Self::with_sign(diff)
        ))
    }

    /// 數值一律帶正負號，讓方向一眼可辨；`format_decimal_with_commas` 不會替正數補 `+`。
    fn with_sign(value: Decimal) -> String {
        let formatted = format_decimal_with_commas(value);
        if value > Decimal::ZERO {
            format!("+{formatted}")
        } else {
            formatted
        }
    }

    /// 把 yyyyMM 轉成 yyyy\-MM；與除權息通知一樣，訊息裡的日期都寫成人看得懂的格式。
    fn format_revenue_month(date: i64) -> String {
        format!("{}-{:02}", date / 100, date % 100)
    }

    /// 股票代號一律附上 Yahoo 股市連結，與除權息通知的格式一致。
    ///
    /// URL 內的 `.` 是 MarkdownV2 保留字元，必須在字面上就寫成跳脫形式。
    fn stock_link(stock_symbol: &str) -> String {
        format!(
            "[{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0})",
            stock_symbol
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revenue_alert(symbol: &str, name: &str, yoy: Decimal, mom: Decimal) -> HoldingRevenueAlert {
        HoldingRevenueAlert {
            stock_symbol: symbol.to_string(),
            stock_name: name.to_string(),
            monthly: dec!(250000000),
            compared_with_last_month: mom,
            compared_with_last_year_same_month: yoy,
            date: 202608,
        }
    }

    fn financial_alert(last_year_eps: Option<Decimal>) -> HoldingFinancialAlert {
        HoldingFinancialAlert {
            stock_symbol: "2330".to_string(),
            stock_name: "台積電".to_string(),
            quarter: "Q2".to_string(),
            earnings_per_share: dec!(9.56),
            return_on_equity: dec!(8.4),
            gross_profit: dec!(53.1),
            last_year_earnings_per_share: last_year_eps,
            year: 2026,
        }
    }

    #[test]
    fn format_revenue_month_pads_single_digit_month() {
        assert_eq!(EventDispatcher::format_revenue_month(202608), "2026-08");
        assert_eq!(EventDispatcher::format_revenue_month(202612), "2026-12");
    }

    #[test]
    fn with_sign_only_prefixes_positive_values() {
        assert_eq!(EventDispatcher::with_sign(dec!(33.25)), "+33.25");
        assert_eq!(EventDispatcher::with_sign(dec!(-4.1)), "-4.1");
        assert_eq!(EventDispatcher::with_sign(Decimal::ZERO), "0");
    }

    #[test]
    fn revenue_message_lists_every_alert_with_escaped_values() {
        let msg = EventDispatcher::build_revenue_message(
            202608,
            &[
                revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1)),
                revenue_alert("2454", "聯發科", dec!(-25.5), dec!(1.2)),
            ],
        );

        assert!(msg.contains("2026\\-08"), "月份的連字號要跳脫：{msg}");
        assert!(msg.contains("台積電"), "{msg}");
        assert!(msg.contains("聯發科"), "{msg}");
        // 小數點是 MarkdownV2 保留字元，沒跳脫整則訊息會被 Bot API 退回。
        assert!(msg.contains("\\+33\\.25"), "{msg}");
        assert!(msg.contains("\\-25\\.5"), "{msg}");
    }

    #[test]
    fn financial_message_includes_year_over_year_comparison() {
        let msg = EventDispatcher::build_financial_message(
            2026,
            "Q2",
            &[financial_alert(Some(dec!(7.01)))],
        );

        assert!(msg.contains("去年同季"), "{msg}");
        assert!(msg.contains("7\\.01"), "{msg}");
        assert!(msg.contains("\\+2\\.55"), "差額要帶正號並跳脫：{msg}");
    }

    // 查不到去年同季時不能顯示 0，那看起來像獲利歸零。
    #[test]
    fn financial_message_omits_comparison_when_last_year_is_missing() {
        let msg = EventDispatcher::build_financial_message(2026, "Q2", &[financial_alert(None)]);

        assert!(!msg.contains("去年同季"), "{msg}");
        assert!(msg.contains("9\\.56"), "{msg}");
    }

    #[test]
    fn stock_link_escapes_dots_in_url() {
        let link = EventDispatcher::stock_link("2330");

        assert_eq!(link, "[2330](https://tw\\.stock\\.yahoo\\.com/quote/2330)");
    }
}
