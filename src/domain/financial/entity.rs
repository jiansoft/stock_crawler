use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// 財務報表領域實體。
///
/// 封裝公司在特定年度與季度的財務數據（如毛利率、營業利益率、EPS 等）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinancialStatement {
    /// 序號
    pub serial: i64,
    /// 股票代號
    pub security_code: String,
    /// 年度
    pub year: i64,
    /// 季度 (Q1, Q2, Q3, Q4)
    pub quarter: String,
    /// 營業毛利率 (%)
    pub gross_profit: Decimal,
    /// 營業利益率 (%)
    pub operating_profit_margin: Decimal,
    /// 稅前淨利率 (%)
    pub pre_tax_income: Decimal,
    /// 稅後淨利率 (%)
    pub net_income: Decimal,
    /// 每股淨值 (元)
    pub net_asset_value_per_share: Decimal,
    /// 每股營收 (元)
    pub sales_per_share: Decimal,
    /// 每股稅後淨利 (EPS, 元)
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利 (元)
    pub profit_before_tax: Decimal,
    /// 股東權益報酬率 (ROE, %)
    pub return_on_equity: Decimal,
    /// 資產報酬率 (ROA, %)
    pub return_on_assets: Decimal,
    /// 建立時間
    pub created_time: DateTime<Local>,
    /// 最後更新時間
    pub updated_time: DateTime<Local>,
}

impl Default for FinancialStatement {
    fn default() -> Self {
        FinancialStatement {
            serial: 0,
            security_code: String::new(),
            year: 0,
            quarter: String::new(),
            gross_profit: Decimal::ZERO,
            operating_profit_margin: Decimal::ZERO,
            pre_tax_income: Decimal::ZERO,
            net_income: Decimal::ZERO,
            net_asset_value_per_share: Decimal::ZERO,
            sales_per_share: Decimal::ZERO,
            earnings_per_share: Decimal::ZERO,
            profit_before_tax: Decimal::ZERO,
            return_on_equity: Decimal::ZERO,
            return_on_assets: Decimal::ZERO,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }
}

impl FinancialStatement {
    /// 建立指定股票代號的財報模型。
    pub fn new(security_code: String) -> Self {
        FinancialStatement {
            security_code,
            ..Default::default()
        }
    }
}

/// 月營收領域實體。
///
/// 封裝公司在特定月份的營收與價格區間數據。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthlyRevenue {
    /// 股票代號
    pub stock_symbol: String,
    /// 當月營收 (元)
    pub monthly: Decimal,
    /// 上月營收 (元)
    pub last_month: Decimal,
    /// 去年當月營收 (元)
    pub last_year_this_month: Decimal,
    /// 當月累計營收 (元)
    pub monthly_accumulated: Decimal,
    /// 去年累計營收 (元)
    pub last_year_monthly_accumulated: Decimal,
    /// 上月比較增減 (%)
    pub compared_with_last_month: Decimal,
    /// 去年同月增減 (%)
    pub compared_with_last_year_same_month: Decimal,
    /// 前期比較增減 (%)
    pub accumulated_compared_with_last_year: Decimal,
    /// 月均價 (元)
    pub avg_price: Decimal,
    /// 當月最低價 (元)
    pub lowest_price: Decimal,
    /// 當月最高價 (元)
    pub highest_price: Decimal,
    /// 營收代表之日期 (timestamp，以 Unix 時間戳記表示，通常對齊至當月 1 日)
    pub date: i64,
    /// 建立時間
    pub create_time: DateTime<Local>,
}

impl Default for MonthlyRevenue {
    fn default() -> Self {
        MonthlyRevenue {
            stock_symbol: String::new(),
            monthly: Decimal::ZERO,
            last_month: Decimal::ZERO,
            last_year_this_month: Decimal::ZERO,
            monthly_accumulated: Decimal::ZERO,
            last_year_monthly_accumulated: Decimal::ZERO,
            compared_with_last_month: Decimal::ZERO,
            compared_with_last_year_same_month: Decimal::ZERO,
            accumulated_compared_with_last_year: Decimal::ZERO,
            avg_price: Decimal::ZERO,
            lowest_price: Decimal::ZERO,
            highest_price: Decimal::ZERO,
            date: 0,
            create_time: Local::now(),
        }
    }
}

impl MonthlyRevenue {
    /// 建立月營收實體預設值。
    pub fn new() -> Self {
        Default::default()
    }
}

/// 個股估值領域實體。
///
/// 彙整價格區間、股利法、EPS 法、PBR 法與 PER 法等估值結果。
#[derive(Debug, Clone, PartialEq)]
pub struct PriceEstimate {
    /// 估值日期
    pub date: NaiveDate,
    /// 參考的最後一筆日報價日期
    pub last_daily_quote_date: String,
    /// 股票代號
    pub security_code: String,
    /// 股票名稱
    pub name: String,
    /// 當日收盤價 (元)
    pub closing_price: f64,
    /// 估值百分比 (收盤價相對便宜價)
    pub percentage: f64,
    /// 加權便宜價 (元)
    pub cheap: f64,
    /// 加權合理價 (元)
    pub fair: f64,
    /// 加權昂貴價 (元)
    pub expensive: f64,
    /// 價格法便宜價 (元)
    pub price_cheap: f64,
    /// 價格法合理價 (元)
    pub price_fair: f64,
    /// 價格法昂貴價 (元)
    pub price_expensive: f64,
    /// 股利法便宜價 (元)
    pub dividend_cheap: f64,
    /// 股利法合理價 (元)
    pub dividend_fair: f64,
    /// 股利法昂貴價 (元)
    pub dividend_expensive: f64,
    /// EPS 法便宜價 (元)
    pub eps_cheap: f64,
    /// EPS 法合理價 (元)
    pub eps_fair: f64,
    /// EPS 法昂貴價 (元)
    pub eps_expensive: f64,
    /// PBR 法便宜價 (元)
    pub pbr_cheap: f64,
    /// PBR 法合理價 (元)
    pub pbr_fair: f64,
    /// PBR 法昂貴價 (元)
    pub pbr_expensive: f64,
    /// 參與統計的年度數
    pub year_count: i32,
    /// 內部排序或索引欄位
    pub index: i32,
}

impl Default for PriceEstimate {
    fn default() -> Self {
        PriceEstimate {
            date: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            last_daily_quote_date: String::new(),
            security_code: String::new(),
            name: String::new(),
            closing_price: 0.0,
            percentage: 0.0,
            cheap: 0.0,
            fair: 0.0,
            expensive: 0.0,
            price_cheap: 0.0,
            price_fair: 0.0,
            price_expensive: 0.0,
            dividend_cheap: 0.0,
            dividend_fair: 0.0,
            dividend_expensive: 0.0,
            eps_cheap: 0.0,
            eps_fair: 0.0,
            eps_expensive: 0.0,
            pbr_cheap: 0.0,
            pbr_fair: 0.0,
            pbr_expensive: 0.0,
            year_count: 0,
            index: 0,
        }
    }
}

impl PriceEstimate {
    /// 建立個股估值實體。
    pub fn new(security_code: String, date: NaiveDate) -> Self {
        PriceEstimate {
            security_code,
            date,
            ..Default::default()
        }
    }
}

/// 持股月營收摘要領域實體。
///
/// 每月營收公布後為**每一檔持股**產生一筆，供 Telegram 通知使用；
/// 不再只挑年增率超過門檻的個股——持股數量有限，全列出來才能一次看完整個組合的動能。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingRevenueAlert {
    /// 股票代號
    pub stock_symbol: String,
    /// 股票名稱
    pub stock_name: String,
    /// 產業分類名稱；查無分類（`stock_industry_id` 未對應）時為 None
    pub industry_name: Option<String>,
    /// 當月營收 (千元)
    pub monthly: Decimal,
    /// 當月累計營收 (千元)，自當年 1 月起算
    pub monthly_accumulated: Decimal,
    /// 上月比較增減 (%)
    pub compared_with_last_month: Decimal,
    /// 去年同月增減 (%)
    pub compared_with_last_year_same_month: Decimal,
    /// 累計營收較去年同期增減 (%)
    pub accumulated_compared_with_last_year: Decimal,
    /// 發行股數 (股)；尚未回補時為 0
    pub issued_share: i64,
    /// 近四季平均稅後淨利率 (%)；查無可用財報時為 None
    pub net_income_margin: Option<Decimal>,
    /// 今年自 Q1 起連續公布、且季底不晚於本期的季報 EPS 加總 (元)；無可用錨點時為 None
    pub anchor_eps: Option<Decimal>,
    /// 上述錨點季底當月的累計營收 (千元)；無可用錨點時為 None
    pub anchor_accumulated_revenue: Option<Decimal>,
    /// 營收月份 (yyyyMM)
    pub date: i64,
}

/// 台股月營收以千元為單位；要對上「元／股」的 EPS 必須先換算成元。
const REVENUE_THOUSANDS_TO_NTD: Decimal = dec!(1000);

/// 百分比換算成比率的除數。
const PERCENT: Decimal = dec!(100);

/// 一年的月數，用於把累計 EPS 年化。
const MONTHS_PER_YEAR: Decimal = dec!(12);

/// 推估 EPS 的計算依據。
///
/// 兩種方法的可信度差很多，通知必須讓人一眼分得出來用的是哪一種。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpsEstimateBasis {
    /// 以今年已公布的實際季報 EPS 為錨點，只把尚未公布的月份按營收比例外推。
    ReportedQuarters,
    /// 以近四季平均稅後淨利率套用在累計營收上；今年還沒有任何季報可錨定時的退路。
    NetIncomeMargin,
}

/// 由累計營收推估出來的 EPS。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpsEstimate {
    /// 今年累計推估 EPS (元)
    pub accumulated: Decimal,
    /// 依已公布月數線性年化後的全年推估 EPS (元)
    pub annual: Decimal,
    /// 計算依據
    pub basis: EpsEstimateBasis,
}

impl HoldingRevenueAlert {
    /// 累計營收所涵蓋的月數。
    ///
    /// 台股的「當月累計營收」一律自當年 1 月起算，因此月份數字本身就是月數。
    pub fn accumulated_months(&self) -> i64 {
        self.date % 100
    }

    /// 由目前累計營收推估今年的 EPS。
    ///
    /// 優先用 [`EpsEstimateBasis::ReportedQuarters`]，查不到錨點才退回
    /// [`EpsEstimateBasis::NetIncomeMargin`]，兩者都算不出來時回傳 `None`
    /// ——寧可不顯示，也不要把「查無資料」畫成 0。
    ///
    /// 不論用哪一種方法，數字都會隨每月營收公布而變動，這是預期行為而不是資料不穩。
    pub fn estimate_eps(&self) -> Option<EpsEstimate> {
        let months = self.accumulated_months();
        if !(1..=12).contains(&months) {
            return None;
        }

        let (accumulated, basis) = self
            .anchored_accumulated_eps()
            .map(|eps| (eps, EpsEstimateBasis::ReportedQuarters))
            .or_else(|| {
                self.margin_accumulated_eps()
                    .map(|eps| (eps, EpsEstimateBasis::NetIncomeMargin))
            })?;

        Some(EpsEstimate {
            accumulated,
            annual: accumulated * MONTHS_PER_YEAR / Decimal::from(months),
            basis,
        })
    }

    /// 錨定法：`今年已公布的實際累計 EPS × (本期累計營收 ÷ 錨點季底累計營收)`。
    ///
    /// 已實現的部分直接採用財報數字，只有尚未公布的一兩個月才用營收比例外推，
    /// 因此誤差被限縮在那幾個月裡。比率的分子分母都是同一檔股票的營收，
    /// 金控、保險這類「營收」與淨利率基準對不起來的產業也能算對。
    ///
    /// 缺錨點（今年還沒公布季報、季別不連續、或查不到該季底的營收）時回傳 `None`。
    fn anchored_accumulated_eps(&self) -> Option<Decimal> {
        let anchor_eps = self.anchor_eps?;
        let anchor_revenue = self.anchor_accumulated_revenue?;
        if anchor_revenue <= Decimal::ZERO {
            return None;
        }

        Some(anchor_eps * self.monthly_accumulated / anchor_revenue)
    }

    /// 淨利率法：`累計營收(元) ÷ 發行股數 × 近四季平均稅後淨利率`。
    ///
    /// 只在今年還沒有任何季報可錨定時使用。台股季報的公布期限比月營收晚（Q1 約 5 月中、
    /// Q2 約 8 月中），所以實務上 1~4 月的營收通知會走這條路。淨利率取的是
    /// **近四季未加權平均**，一次性損益或季節性都沒有反映，金融保險業的營收
    /// 與淨利率基準也對不齊，因此誤差可能很大——通知會標成「概估」以示區別。
    fn margin_accumulated_eps(&self) -> Option<Decimal> {
        let margin = self.net_income_margin?;
        if self.issued_share <= 0 || margin.is_zero() {
            return None;
        }

        let shares = Decimal::from(self.issued_share);

        Some(self.monthly_accumulated * REVENUE_THOUSANDS_TO_NTD / shares * margin / PERCENT)
    }
}

/// 持股財報摘要領域實體。
///
/// 供季報公布後的 Telegram 通知使用。`last_year_earnings_per_share` 讓通知能直接呈現
/// 與去年同季的差距——季度之間有季節性，跟上一季比並沒有意義。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingFinancialAlert {
    /// 股票代號
    pub stock_symbol: String,
    /// 股票名稱
    pub stock_name: String,
    /// 季度 (Q1, Q2, Q3, Q4)
    pub quarter: String,
    /// 每股稅後淨利 (元)
    pub earnings_per_share: Decimal,
    /// 股東權益報酬率 (%)
    pub return_on_equity: Decimal,
    /// 營業毛利率 (%)
    pub gross_profit: Decimal,
    /// 去年同季的每股稅後淨利 (元)；查無該期財報時為 None
    pub last_year_earnings_per_share: Option<Decimal>,
    /// 年度
    pub year: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建立一筆兩種推估法都算得出來的樣本。
    ///
    /// 累計 1,000,000 千元＝10 億元；股數 1 億股 ⇒ 每股累計營收 10 元。
    /// 錨定法：錨點 EPS 3 元、錨點營收 600,000 千元 ⇒ 3 × (1,000,000 ÷ 600,000) ＝ 5 元。
    /// 淨利率法：10 元 × 40% ＝ 4 元。兩者刻意給不同答案，才能分辨用了哪一條路徑。
    fn revenue_alert(date: i64) -> HoldingRevenueAlert {
        HoldingRevenueAlert {
            stock_symbol: "2330".to_string(),
            stock_name: "台積電".to_string(),
            industry_name: Some("半導體業".to_string()),
            monthly: dec!(514805337),
            monthly_accumulated: dec!(1000000),
            compared_with_last_month: dec!(10.09),
            compared_with_last_year_same_month: dec!(53.32),
            accumulated_compared_with_last_year: dec!(41.2),
            issued_share: 100_000_000,
            net_income_margin: Some(dec!(40)),
            anchor_eps: Some(dec!(3)),
            anchor_accumulated_revenue: Some(dec!(600000)),
            date,
        }
    }

    #[test]
    fn accumulated_months_reads_the_month_part_of_yyyymm() {
        assert_eq!(revenue_alert(202601).accumulated_months(), 1);
        assert_eq!(revenue_alert(202612).accumulated_months(), 12);
    }

    // 有錨點就用錨點，不會退回淨利率法（4 元）。
    #[test]
    fn estimate_eps_prefers_reported_quarters() {
        let estimate = revenue_alert(202608).estimate_eps().expect("estimate");

        assert_eq!(estimate.basis, EpsEstimateBasis::ReportedQuarters);
        assert_eq!(estimate.accumulated, dec!(5));
        // 8 個月做出 5 元，線性外推全年 7.5 元。
        assert_eq!(estimate.annual, dec!(7.5));
    }

    // 今年還沒公布季報（1~3 月）時退回淨利率法。
    #[test]
    fn estimate_eps_falls_back_to_net_income_margin() {
        let mut alert = revenue_alert(202603);
        alert.anchor_eps = None;
        alert.anchor_accumulated_revenue = None;

        let estimate = alert.estimate_eps().expect("estimate");

        assert_eq!(estimate.basis, EpsEstimateBasis::NetIncomeMargin);
        assert_eq!(estimate.accumulated, dec!(4));
        assert_eq!(estimate.annual, dec!(16));
    }

    // 錨點營收為 0 會讓比率除以零，必須當成沒有錨點而退回淨利率法。
    #[test]
    fn estimate_eps_falls_back_when_anchor_revenue_is_zero() {
        let mut alert = revenue_alert(202608);
        alert.anchor_accumulated_revenue = Some(Decimal::ZERO);

        let estimate = alert.estimate_eps().expect("estimate");

        assert_eq!(estimate.basis, EpsEstimateBasis::NetIncomeMargin);
        assert_eq!(estimate.accumulated, dec!(4));
    }

    // 錨點季底就是本期時比率為 1，推估值等於實際公布的累計 EPS。
    #[test]
    fn estimate_eps_equals_reported_eps_at_quarter_end() {
        let mut alert = revenue_alert(202606);
        alert.monthly_accumulated = dec!(600000);

        let estimate = alert.estimate_eps().expect("estimate");

        assert_eq!(estimate.accumulated, dec!(3));
    }

    // 12 月的累計就是全年，年化後不應該再被放大。
    #[test]
    fn estimate_eps_annual_equals_accumulated_in_december() {
        let estimate = revenue_alert(202612).estimate_eps().expect("estimate");

        assert_eq!(estimate.annual, estimate.accumulated);
    }

    // 虧損公司的錨點 EPS 是負的，推估值必須跟著是負的，不能被當成沒資料。
    #[test]
    fn estimate_eps_keeps_negative_anchor() {
        let mut alert = revenue_alert(202608);
        alert.anchor_eps = Some(dec!(-1.2));

        let estimate = alert.estimate_eps().expect("estimate");

        assert_eq!(estimate.accumulated, dec!(-2));
    }

    // 兩種方法都缺料時不推估。
    #[test]
    fn estimate_eps_is_none_without_any_usable_input() {
        let mut alert = revenue_alert(202608);
        alert.anchor_eps = None;
        alert.anchor_accumulated_revenue = None;
        alert.net_income_margin = None;

        assert_eq!(alert.estimate_eps(), None);

        let mut alert = revenue_alert(202608);
        alert.anchor_eps = None;
        alert.anchor_accumulated_revenue = None;
        alert.issued_share = 0;

        assert_eq!(alert.estimate_eps(), None);
    }

    // 月份欄位異常（yyyyMM 的月份不在 1~12）時不推估，避免除以 0。
    #[test]
    fn estimate_eps_is_none_for_invalid_month() {
        assert_eq!(revenue_alert(202600).estimate_eps(), None);
    }
}
