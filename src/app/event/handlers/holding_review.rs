//! 持股定期回顧通知：月營收公布與季報公布後各發一則彙總訊息。
//!
//! 兩個事件的處理流程一樣——收到「某期資料已更新」後自己去查持股、組訊息、送出——
//! 因此放在同一個模組共用排版工具。
//!
//! 只看**目前持有**的股票。全市場每個月有上千檔公布營收，全推等於製造雜訊；
//! 這兩則通知的用途是「我手上的股票發生了什麼」，不是市場掃描。
//!
//! 月營收通知列出**每一檔**持股（不設年增率門檻）並依年增率由高到低排序，
//! 讓整個組合的動能在同一則訊息裡一次排開，便於決定加碼或減持。

use std::fmt::Write;

use anyhow::Result;
use rust_decimal::Decimal;

use crate::app::event::taiwan_stock::format_decimal_with_commas;
// 通知走 core::alert（port），跳脫工具走 core::util::text——
// app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向。
use crate::core::{alert, util::text};
use crate::domain::financial::entity::{
    EpsEstimateBasis, HoldingFinancialAlert, HoldingRevenueAlert,
};
use crate::domain::financial::repository::FinancialRepository;
use crate::infra::database::repository::financial::PgFinancialRepository;

use super::EventDispatcher;

impl EventDispatcher {
    /// 處理 `MonthlyRevenueUpdated` 事件：推播全部持股的月營收。
    pub(super) async fn handle_monthly_revenue_updated(date: i64) -> Result<()> {
        let repo = PgFinancialRepository::new();
        let alerts = repo.fetch_holding_revenue_alerts(date).await?;

        if alerts.is_empty() {
            tracing::info!("持股月營收通知：{} 沒有任何持股公布月營收", date);
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
    ///
    /// 每檔兩行：第一行是當月數字（產業別放在股名前，方便一眼看出族群是否同步轉強），
    /// 第二行是累計數字與由累計營收推估的 EPS。
    fn build_revenue_message(date: i64, alerts: &[HoldingRevenueAlert]) -> String {
        // 每檔兩行、每行約 60~80 個字元，先抓一個不會反覆擴張的容量。
        let mut msg = String::with_capacity(alerts.len() * 192 + 64);
        let _ = writeln!(
            &mut msg,
            "{} 持股月營收（共 {} 檔，依年增率排序；營收單位：千元）︰",
            text::escape_markdown_v2(Self::format_revenue_month(date)),
            alerts.len()
        );

        for alert in alerts {
            let _ = writeln!(
                &mut msg,
                "    {} {}{} 營收︰{} 年增︰{}% 月增︰{}%",
                Self::stock_link(&alert.stock_symbol),
                Self::format_industry(alert),
                text::escape_markdown_v2(&alert.stock_name),
                text::escape_markdown_v2(format_decimal_with_commas(alert.monthly)),
                text::escape_markdown_v2(Self::with_sign(alert.compared_with_last_year_same_month)),
                text::escape_markdown_v2(Self::with_sign(alert.compared_with_last_month))
            );
            let _ = writeln!(
                &mut msg,
                "        累計︰{} 累計年增︰{}%{}",
                text::escape_markdown_v2(format_decimal_with_commas(alert.monthly_accumulated)),
                text::escape_markdown_v2(Self::with_sign(
                    alert.accumulated_compared_with_last_year
                )),
                Self::format_estimated_eps(alert)
            );
        }

        msg
    }

    /// 產業分類名稱，後面補一個空白接在股名前；查無分類時回傳空字串。
    fn format_industry(alert: &HoldingRevenueAlert) -> String {
        match &alert.industry_name {
            Some(name) if !name.is_empty() => format!("{} ", text::escape_markdown_v2(name)),
            _ => String::new(),
        }
    }

    /// 組出「 推估EPS︰X（年估 Y）」這段文字；兩種推估法都算不出來時回傳空字串。
    ///
    /// 與去年同季 EPS 的處理一致：沒有可用資料時寧可整段不顯示，也不要把查無資料畫成 0。
    /// 12 月的累計本身就是全年，年估與累計相同，此時不再重複列出。
    ///
    /// 用「推估／概估」兩個詞區分計算依據——概估是今年還沒有季報可錨定時的退路，
    /// 誤差比推估大一個量級，混在一起看會做出錯誤的加減碼判斷。
    fn format_estimated_eps(alert: &HoldingRevenueAlert) -> String {
        let Some(estimate) = alert.estimate_eps() else {
            return String::new();
        };

        let label = match estimate.basis {
            EpsEstimateBasis::ReportedQuarters => "推估EPS",
            EpsEstimateBasis::NetIncomeMargin => "概估EPS",
        };
        let annual = if alert.accumulated_months() < 12 {
            format!("（年估 {}）", format_decimal_with_commas(estimate.annual))
        } else {
            String::new()
        };

        text::escape_markdown_v2(format!(
            " {label}︰{}{annual}",
            format_decimal_with_commas(estimate.accumulated)
        ))
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
    use rust_decimal_macros::dec;

    use super::*;

    fn revenue_alert(symbol: &str, name: &str, yoy: Decimal, mom: Decimal) -> HoldingRevenueAlert {
        HoldingRevenueAlert {
            stock_symbol: symbol.to_string(),
            stock_name: name.to_string(),
            industry_name: Some("半導體業".to_string()),
            monthly: dec!(250000000),
            monthly_accumulated: dec!(1000000),
            compared_with_last_month: mom,
            compared_with_last_year_same_month: yoy,
            accumulated_compared_with_last_year: dec!(12.5),
            issued_share: 100_000_000,
            net_income_margin: Some(dec!(40)),
            // 錨點 EPS 3 元、錨點營收 600,000 千元 ⇒ 3 × (1,000,000 ÷ 600,000) ＝ 5 元。
            anchor_eps: Some(dec!(3)),
            anchor_accumulated_revenue: Some(dec!(600000)),
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
    fn revenue_message_puts_industry_before_stock_name() {
        let msg = EventDispatcher::build_revenue_message(
            202608,
            &[revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1))],
        );

        assert!(msg.contains("半導體業 台積電"), "產業別要接在股名前：{msg}");
    }

    // 查無產業分類時整段省略，不留下多餘空白或空字串。
    #[test]
    fn revenue_message_omits_industry_when_unknown() {
        let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
        alert.industry_name = None;

        let msg = EventDispatcher::build_revenue_message(202608, &[alert]);

        assert!(msg.contains(") 台積電 營收︰"), "{msg}");
    }

    // 錨點 EPS 3 元 × (累計 1,000,000 ÷ 錨點 600,000) ＝ 5 元；8 個月年化 ＝ 7.5 元。
    #[test]
    fn revenue_message_shows_estimated_eps_from_accumulated_revenue() {
        let msg = EventDispatcher::build_revenue_message(
            202608,
            &[revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1))],
        );

        assert!(msg.contains("累計︰1,000,000"), "{msg}");
        assert!(msg.contains(r"累計年增︰\+12\.5%"), "{msg}");
        assert!(msg.contains(r"推估EPS︰5（年估 7\.5）"), "{msg}");
    }

    // 今年還沒公布季報時退回淨利率法，標籤要換成「概估」以示區別。
    #[test]
    fn revenue_message_labels_margin_fallback_as_rough_estimate() {
        let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
        alert.anchor_eps = None;
        alert.anchor_accumulated_revenue = None;

        let msg = EventDispatcher::build_revenue_message(202608, &[alert]);

        assert!(msg.contains("概估EPS︰4"), "{msg}");
        assert!(!msg.contains("推估EPS"), "{msg}");
    }

    // 兩種推估法都缺料時整段省略，不要把「查無資料」畫成 0。
    #[test]
    fn revenue_message_omits_estimated_eps_when_data_is_missing() {
        let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
        alert.anchor_eps = None;
        alert.anchor_accumulated_revenue = None;
        alert.net_income_margin = None;

        let msg = EventDispatcher::build_revenue_message(202608, &[alert]);

        assert!(!msg.contains("估EPS"), "{msg}");
    }

    // 12 月的累計就是全年，年估與累計相同時不再重複列出。
    #[test]
    fn revenue_message_omits_annual_estimate_in_december() {
        let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
        alert.date = 202612;

        let msg = EventDispatcher::build_revenue_message(202612, &[alert]);

        assert!(msg.contains("推估EPS︰5"), "{msg}");
        assert!(!msg.contains("年估"), "{msg}");
    }

    // 排序由 SQL 決定，訊息必須原封不動地照傳入順序輸出。
    #[test]
    fn revenue_message_keeps_input_order() {
        let msg = EventDispatcher::build_revenue_message(
            202608,
            &[
                revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1)),
                revenue_alert("2454", "聯發科", dec!(-25.5), dec!(1.2)),
            ],
        );

        let tsmc = msg.find("台積電").expect("台積電 應該在訊息中");
        let mtk = msg.find("聯發科").expect("聯發科 應該在訊息中");
        assert!(tsmc < mtk, "{msg}");
    }

    // 標題要標明是全部持股與排序方式，不再出現舊版的年增率門檻。
    #[test]
    fn revenue_message_header_reports_count_and_sorting() {
        let msg = EventDispatcher::build_revenue_message(
            202608,
            &[
                revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1)),
                revenue_alert("2454", "聯發科", dec!(-25.5), dec!(1.2)),
            ],
        );

        assert!(msg.contains("共 2 檔，依年增率排序"), "{msg}");
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
