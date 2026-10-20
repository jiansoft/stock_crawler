//! 持股月營收、季報通知的訊息組裝與格式化（新增持股標記、年增率與三率、推估 EPS、股票連結）。

use super::*;

impl EventDispatcher {
    /// 補發通知時，新增持股前面要加的記號；不是新增或首次通知時回傳空字串。
    pub(super) fn new_mark(
        stock_symbol: &str,
        new_symbols: Option<&BTreeSet<String>>,
    ) -> &'static str {
        match new_symbols {
            Some(symbols) if symbols.contains(stock_symbol) => NEW_HOLDING_MARK,
            _ => "",
        }
    }

    /// 組出月營收通知訊息。
    ///
    /// 每檔兩行：第一行是當月數字（產業別放在股名前，方便一眼看出族群是否同步轉強），
    /// 第二行是累計數字與由累計營收推估的 EPS。
    pub(super) fn build_revenue_message(
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
                "    {}{} {}{} 營收︰{} 年增︰{}% 月增︰{}%{}",
                Self::new_mark(&alert.stock_symbol, new_symbols),
                Self::stock_link(&alert.stock_symbol),
                Self::format_industry(alert),
                text::escape_markdown_v2(&alert.stock_name),
                text::escape_markdown_v2(format_decimal_with_commas(alert.monthly)),
                text::escape_markdown_v2(Self::with_sign(alert.compared_with_last_year_same_month)),
                text::escape_markdown_v2(Self::with_sign(alert.compared_with_last_month)),
                Self::revenue_marks(alert)
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

    /// 當月營收的標記：創歷史新高（🏆），或連續年減達持股健檢門檻（📉連續 N 月年減）；都沒有時回傳空字串。
    pub(super) fn revenue_marks(alert: &HoldingRevenueAlert) -> String {
        if alert.is_record_high() {
            return " 🏆創歷史新高".to_owned();
        }
        if alert.yoy_decline_months >= i64::from(REVENUE_DECLINE_WARNING_MONTHS) {
            return format!(" 📉連續 {} 月年減", alert.yoy_decline_months);
        }
        String::new()
    }

    /// 產業分類名稱，後面補一個空白接在股名前；查無分類時回傳空字串。
    pub(super) fn format_industry(alert: &HoldingRevenueAlert) -> String {
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
    pub(super) fn format_estimated_eps(alert: &HoldingRevenueAlert) -> String {
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
    pub(super) fn build_financial_message(
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
                "    {}{} {} EPS︰{}元{} ROE︰{}% {}",
                Self::new_mark(&alert.stock_symbol, new_symbols),
                Self::stock_link(&alert.stock_symbol),
                text::escape_markdown_v2(&alert.stock_name),
                text::escape_markdown_v2(format_decimal_with_commas(alert.earnings_per_share)),
                Self::format_eps_comparison(alert),
                text::escape_markdown_v2(format_decimal_with_commas(alert.return_on_equity)),
                Self::format_margins(alert)
            );
        }

        msg
    }

    /// 組出「（去年同季 X，±Y）」這段比較文字；查不到去年同季時回傳空字串。
    ///
    /// 沒有可比對象時寧可不顯示，也不要把「查無資料」畫成 0 —— 那會看起來像獲利歸零。
    pub(super) fn format_eps_comparison(alert: &HoldingFinancialAlert) -> String {
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

    /// 三率與去年同季的比較：「毛利率︰53.1%（+2.3） 營益率︰…（…） 淨利率︰…（…）」，三率同向時再標
    /// 三率三升／三率三降。毛利率一律列出（與過去的通知相同）；營益率、淨利率為 0（來源沒有提供）時
    /// 不列；查不到去年同季的那一率不附括號。
    pub(super) fn format_margins(alert: &HoldingFinancialAlert) -> String {
        let [gross, operating, net] = alert.margin_changes();
        let item = |label: &str, value: Decimal, change: Option<Decimal>| {
            let change = change
                .map(|change| format!("（{}）", Self::with_sign(change)))
                .unwrap_or_default();
            format!("{label}︰{}%{change}", format_decimal_with_commas(value))
        };
        let mut parts = vec![item("毛利率", alert.gross_profit, gross)];
        if !alert.operating_margin.is_zero() {
            parts.push(item("營益率", alert.operating_margin, operating));
        }
        if !alert.net_margin.is_zero() {
            parts.push(item("淨利率", alert.net_margin, net));
        }
        match alert.margin_trend() {
            Some(MarginTrend::AllUp) => parts.push("📈三率三升".to_owned()),
            Some(MarginTrend::AllDown) => parts.push("📉三率三降".to_owned()),
            Some(MarginTrend::Mixed) | None => {}
        }
        text::escape_markdown_v2(parts.join(" "))
    }

    /// 數值一律帶正負號，讓方向一眼可辨；`format_decimal_with_commas` 不會替正數補 `+`。
    pub(super) fn with_sign(value: Decimal) -> String {
        let formatted = format_decimal_with_commas(value);
        if value > Decimal::ZERO {
            format!("+{formatted}")
        } else {
            formatted
        }
    }

    /// 把 yyyyMM 轉成 yyyy\-MM；與除權息通知一樣，訊息裡的日期都寫成人看得懂的格式。
    pub(super) fn format_revenue_month(date: i64) -> String {
        format!("{}-{:02}", date / 100, date % 100)
    }

    /// 股票代號一律附上 Yahoo 股市連結，與除權息通知的格式一致。
    ///
    /// URL 內的 `.` 是 MarkdownV2 保留字元，必須在字面上就寫成跳脫形式。
    pub(super) fn stock_link(stock_symbol: &str) -> String {
        format!(
            "[{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0})",
            stock_symbol
        )
    }
}
