//! 持股季報通知：EPS、ROE 與三率對去年同季的變化。

use rust_decimal::Decimal;

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
    /// 營業利益率 (%)；0 代表來源沒有提供
    pub operating_margin: Decimal,
    /// 稅後淨利率 (%)；0 代表來源沒有提供
    pub net_margin: Decimal,
    /// 去年同季的每股稅後淨利 (元)；查無該期財報時為 None
    pub last_year_earnings_per_share: Option<Decimal>,
    /// 去年同季的營業毛利率 (%)；查無該期財報時為 None
    pub last_year_gross_profit: Option<Decimal>,
    /// 去年同季的營業利益率 (%)；查無該期財報時為 None
    pub last_year_operating_margin: Option<Decimal>,
    /// 去年同季的稅後淨利率 (%)；查無該期財報時為 None
    pub last_year_net_margin: Option<Decimal>,
    /// 年度
    pub year: i32,
}

/// 毛利率、營業利益率、稅後淨利率三率與去年同季相比的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginTrend {
    /// 三率都比去年同季高（三率三升）。
    AllUp,
    /// 三率都比去年同季低（三率三降）。
    AllDown,
    /// 有升有降或持平。
    Mixed,
}

impl MarginTrend {
    /// 依三率對去年同季的變化（正為上升）判斷方向；任一率無法比較（`None`）時為 `None`。
    pub fn classify<T: PartialOrd + Default>(changes: [Option<T>; 3]) -> Option<Self> {
        let changes: Vec<T> = changes.into_iter().collect::<Option<_>>()?;
        let zero = T::default();
        Some(if changes.iter().all(|change| *change > zero) {
            Self::AllUp
        } else if changes.iter().all(|change| *change < zero) {
            Self::AllDown
        } else {
            Self::Mixed
        })
    }
}

impl HoldingFinancialAlert {
    /// 三率（毛利率、營業利益率、稅後淨利率）對去年同季的變化（百分點），依序排列；某一率本季或
    /// 去年同季查不到、或為 0（來源沒有提供）時該項為 `None`。
    pub fn margin_changes(&self) -> [Option<Decimal>; 3] {
        let change = |current: Decimal, last_year: Option<Decimal>| {
            last_year
                .filter(|last_year| !last_year.is_zero() && !current.is_zero())
                .map(|last_year| current - last_year)
        };
        [
            change(self.gross_profit, self.last_year_gross_profit),
            change(self.operating_margin, self.last_year_operating_margin),
            change(self.net_margin, self.last_year_net_margin),
        ]
    }

    /// 三率與去年同季相比的方向；任一率無法比較時為 `None`。
    pub fn margin_trend(&self) -> Option<MarginTrend> {
        MarginTrend::classify(self.margin_changes())
    }
}
