//! 持股定期回顧通知：月營收公布、季報公布、外資持股明顯變化時各發一則彙總訊息。
//!
//! 三個事件的處理流程一樣——收到「某期資料已更新」後自己去查持股、組訊息、送出——
//! 因此放在同一個模組共用排版工具。
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
    EpsEstimateBasis, HoldingFinancialAlert, HoldingRevenueAlert,
};
use crate::domain::financial::repository::FinancialRepository;
use crate::domain::foreign_holding::entity::{
    ForeignHoldingDirection, HoldingForeignHoldingAlert, SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS,
};
use crate::domain::foreign_holding::repository::ForeignHoldingRepository;
use crate::infra::database::repository::financial::PgFinancialRepository;
use crate::infra::database::repository::foreign_holding::PgForeignHoldingRepository;
use crate::infra::nosql::redis::{CLIENT, RedisError};

use super::EventDispatcher;

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

    /// 補發通知時，新增持股前面要加的記號；不是新增或首次通知時回傳空字串。
    fn new_mark(stock_symbol: &str, new_symbols: Option<&BTreeSet<String>>) -> &'static str {
        match new_symbols {
            Some(symbols) if symbols.contains(stock_symbol) => NEW_HOLDING_MARK,
            _ => "",
        }
    }

    /// 組出月營收通知訊息。
    ///
    /// 每檔兩行：第一行是當月數字（產業別放在股名前，方便一眼看出族群是否同步轉強），
    /// 第二行是累計數字與由累計營收推估的 EPS。
    fn build_revenue_message(
        date: i64,
        alerts: &[HoldingRevenueAlert],
        new_symbols: Option<&BTreeSet<String>>,
    ) -> String {
        // 每檔兩行、每行約 60~80 個字元，先抓一個不會反覆擴張的容量。
        let mut msg = String::with_capacity(alerts.len() * 192 + 64);
        let note = new_symbols
            .map(|symbols| format!("，新公布 {} 檔", symbols.len()))
            .unwrap_or_default();
        let _ = writeln!(
            &mut msg,
            "{} 持股月營收（共 {} 檔{}，依年增率排序；營收單位：千元）︰",
            text::escape_markdown_v2(Self::format_revenue_month(date)),
            alerts.len(),
            note
        );

        for alert in alerts {
            let _ = writeln!(
                &mut msg,
                "    {}{} {}{} 營收︰{} 年增︰{}% 月增︰{}%",
                Self::new_mark(&alert.stock_symbol, new_symbols),
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
            EpsEstimateBasis::ReportedQuarters
            | EpsEstimateBasis::ReportedQuartersWithRegression
            | EpsEstimateBasis::ReportedQuartersWithTrailingMargin => "推估EPS",
            EpsEstimateBasis::NetIncomeMargin => "概估EPS",
        };
        let annual = if alert.accumulated_months() < 12 {
            format!("（年估 {}）", format_decimal_with_commas(estimate.annual))
        } else {
            String::new()
        };
        // 營收結構劇變（例如建案入帳）時任何推估法都不準，明講只供參考。
        let caution = if estimate.volatile {
            " ⚠️營收劇變僅供參考"
        } else {
            ""
        };

        text::escape_markdown_v2(format!(
            " {label}︰{}{annual}{caution}",
            format_decimal_with_commas(estimate.accumulated)
        ))
    }

    /// 組出季報通知訊息。
    fn build_financial_message(
        year: i32,
        quarter: &str,
        alerts: &[HoldingFinancialAlert],
        new_symbols: Option<&BTreeSet<String>>,
    ) -> String {
        let mut msg = String::with_capacity(1024);
        let note = new_symbols
            .map(|symbols| format!("（新公布 {} 檔）", symbols.len()))
            .unwrap_or_default();
        let _ = writeln!(
            &mut msg,
            "{} {} 持股財報{}︰",
            text::escape_markdown_v2(year.to_string()),
            text::escape_markdown_v2(quarter),
            note
        );

        for alert in alerts {
            let _ = writeln!(
                &mut msg,
                "    {}{} {} EPS︰{}元{} ROE︰{}% 毛利率︰{}%",
                Self::new_mark(&alert.stock_symbol, new_symbols),
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
            anchor_quarter_no: Some(2),
            ttm_eps: None,
            ttm_revenue: None,
            reg_slope: None,
            reg_intercept: None,
            reg_r2: None,
            reg_quarters: None,
            date: 202608,
        }
    }

    fn foreign_alert(
        symbol: &str,
        name: &str,
        change_20d: Option<Decimal>,
        streak: i32,
    ) -> HoldingForeignHoldingAlert {
        HoldingForeignHoldingAlert {
            stock_symbol: symbol.to_string(),
            stock_name: name.to_string(),
            date: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            share_holding_percentage: dec!(26.3700),
            change_5d: Some(dec!(1.0300)),
            change_20d,
            streak,
        }
    }

    #[test]
    fn foreign_holding_message_groups_by_direction_and_sorts_by_magnitude() {
        let big_up = foreign_alert("6488", "環球晶", Some(dec!(5.2)), 4);
        let small_up = foreign_alert("3105", "穩懋", Some(dec!(3.1)), 1);
        let down = foreign_alert("8069", "元太", Some(dec!(-3.5)), -3);
        let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();

        let msg =
            EventDispatcher::build_foreign_holding_message(date, &[&small_up, &big_up], &[&down]);

        let increase_at = msg.find("外資增持").unwrap();
        let decrease_at = msg.find("外資減持").unwrap();
        assert!(increase_at < decrease_at);
        // 增持依 20 日變化由大到小：環球晶在穩懋前面
        assert!(msg.find("環球晶").unwrap() < msg.find("穩懋").unwrap());
        assert!(msg.contains("20日︰\\+5\\.2"));
        assert!(msg.contains("持股︰26\\.37%"));
        assert!(msg.contains("連續增持 4 日"));
        assert!(msg.contains("連續減持 3 日"));
        // 連續 1 日不註明
        assert_eq!(msg.matches("連續增持").count(), 1);
    }

    #[test]
    fn foreign_holding_message_omits_empty_group() {
        let down = foreign_alert("8069", "元太", Some(dec!(-3.5)), -1);
        let msg = EventDispatcher::build_foreign_holding_message(
            NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            &[],
            &[&down],
        );

        assert!(!msg.contains("外資增持"));
        assert!(msg.contains("外資減持"));
    }

    #[test]
    fn format_optional_change_shows_dash_without_history() {
        assert_eq!(EventDispatcher::format_optional_change(None), "—");
        assert_eq!(
            EventDispatcher::format_optional_change(Some(dec!(1.2))),
            "+1.2"
        );
        assert_eq!(
            EventDispatcher::format_optional_change(Some(dec!(-0.5))),
            "-0.5"
        );
    }

    #[test]
    fn foreign_holding_key_separates_direction_and_symbol() {
        assert_eq!(
            EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Increase, "2330"),
            "holding_review:qfii:up:2330"
        );
        assert_eq!(
            EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Decrease, "2330"),
            "holding_review:qfii:down:2330"
        );
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
            None,
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
            None,
        );

        assert!(msg.contains("半導體業 台積電"), "產業別要接在股名前：{msg}");
    }

    // 查無產業分類時整段省略，不留下多餘空白或空字串。
    #[test]
    fn revenue_message_omits_industry_when_unknown() {
        let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
        alert.industry_name = None;

        let msg = EventDispatcher::build_revenue_message(202608, &[alert], None);

        assert!(msg.contains(") 台積電 營收︰"), "{msg}");
    }

    // 錨點 EPS 3 元 × (累計 1,000,000 ÷ 錨點 600,000) ＝ 5 元；8 個月年化 ＝ 7.5 元。
    #[test]
    fn revenue_message_shows_estimated_eps_from_accumulated_revenue() {
        let msg = EventDispatcher::build_revenue_message(
            202608,
            &[revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1))],
            None,
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

        let msg = EventDispatcher::build_revenue_message(202608, &[alert], None);

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

        let msg = EventDispatcher::build_revenue_message(202608, &[alert], None);

        assert!(!msg.contains("估EPS"), "{msg}");
    }

    // 營收劇變時在推估值後面標註只供參考。
    #[test]
    fn revenue_message_flags_volatile_estimate() {
        let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
        alert.monthly_accumulated = dec!(1200000);

        let msg = EventDispatcher::build_revenue_message(202608, &[alert], None);

        assert!(msg.contains("⚠️營收劇變僅供參考"), "{msg}");
    }

    // 12 月的累計就是全年，年估與累計相同時不再重複列出。
    #[test]
    fn revenue_message_omits_annual_estimate_in_december() {
        let mut alert = revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1));
        alert.date = 202612;

        let msg = EventDispatcher::build_revenue_message(202612, &[alert], None);

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
            None,
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
            None,
        );

        assert!(msg.contains("共 2 檔，依年增率排序"), "{msg}");
    }

    #[test]
    fn financial_message_includes_year_over_year_comparison() {
        let msg = EventDispatcher::build_financial_message(
            2026,
            "Q2",
            &[financial_alert(Some(dec!(7.01)))],
            None,
        );

        assert!(msg.contains("去年同季"), "{msg}");
        assert!(msg.contains("7\\.01"), "{msg}");
        assert!(msg.contains("\\+2\\.55"), "差額要帶正號並跳脫：{msg}");
    }

    // 查不到去年同季時不能顯示 0，那看起來像獲利歸零。
    #[test]
    fn financial_message_omits_comparison_when_last_year_is_missing() {
        let msg =
            EventDispatcher::build_financial_message(2026, "Q2", &[financial_alert(None)], None);

        assert!(!msg.contains("去年同季"), "{msg}");
        assert!(msg.contains("9\\.56"), "{msg}");
    }

    #[test]
    fn parse_notified_symbols_ignores_blank_entries() {
        let symbols = EventDispatcher::parse_notified_symbols("2330, 2454,,");

        assert_eq!(
            symbols,
            BTreeSet::from(["2330".to_string(), "2454".to_string()])
        );
        assert!(EventDispatcher::parse_notified_symbols("").is_empty());
    }

    #[test]
    fn find_new_symbols_excludes_already_notified() {
        let notified = BTreeSet::from(["2330".to_string()]);

        let new_symbols = EventDispatcher::find_new_symbols(&notified, ["2330", "2454"]);

        assert_eq!(new_symbols, BTreeSet::from(["2454".to_string()]));
        assert!(EventDispatcher::find_new_symbols(&notified, ["2330"]).is_empty());
    }

    // 補發時只標出新增的持股，標題補上新增檔數。
    #[test]
    fn revenue_message_marks_new_holdings_on_follow_up() {
        let new_symbols = BTreeSet::from(["2454".to_string()]);
        let msg = EventDispatcher::build_revenue_message(
            202608,
            &[
                revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1)),
                revenue_alert("2454", "聯發科", dec!(-25.5), dec!(1.2)),
            ],
            Some(&new_symbols),
        );

        assert!(msg.contains("共 2 檔，新公布 1 檔"), "{msg}");
        assert!(msg.contains("🆕 [2454]"), "{msg}");
        assert!(!msg.contains("🆕 [2330]"), "{msg}");
    }

    // 首次通知全部都是新的，不需要逐檔標記。
    #[test]
    fn revenue_message_has_no_marks_on_first_notification() {
        let msg = EventDispatcher::build_revenue_message(
            202608,
            &[revenue_alert("2330", "台積電", dec!(33.25), dec!(-4.1))],
            None,
        );

        assert!(!msg.contains('🆕'), "{msg}");
        assert!(!msg.contains("新公布"), "{msg}");
    }

    #[test]
    fn financial_message_marks_new_holdings_on_follow_up() {
        let new_symbols = BTreeSet::from(["2330".to_string()]);
        let msg = EventDispatcher::build_financial_message(
            2026,
            "Q2",
            &[financial_alert(None)],
            Some(&new_symbols),
        );

        assert!(msg.contains("持股財報（新公布 1 檔）︰"), "{msg}");
        assert!(msg.contains("🆕 [2330]"), "{msg}");
    }

    #[test]
    fn stock_link_escapes_dots_in_url() {
        let link = EventDispatcher::stock_link("2330");

        assert_eq!(link, "[2330](https://tw\\.stock\\.yahoo\\.com/quote/2330)");
    }
}
