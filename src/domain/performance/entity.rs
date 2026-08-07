use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 固定投入金額（元）。所有 CAGR 模擬皆以此金額為期初投入。
pub const PRINCIPAL: i64 = 10_000;

/// 期初日對齊的寬限天數。
///
/// 名目期初目標日若無報價（例如該股當時尚未上市、或恰逢停牌），
/// 允許往後找最多這麼多天內的第一筆報價作為實際期初日；
/// 超出此範圍即視為資料不足，不予計算。
pub const BASE_DATE_GRACE_DAYS: i64 = 30;

/// 股票股利的面額（元）。配股率 = 每股股票股利 / 面額。
pub const PAR_VALUE: i64 = 10;

/// CAGR 統計期間。
///
/// 期間長度由 [`CagrPeriod::months`] 定義（以月為單位回推），
/// 實際年數則一律以「期末交易日 − 期初交易日」的日數差計算，
/// 不使用此處的名目長度，避免假日對齊造成系統性偏差。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CagrPeriod {
    /// 3 個月。
    M3,
    /// 6 個月。
    M6,
    /// 1 年。
    Y1,
    /// 1 年 6 個月。
    Y1H,
    /// 2 年。
    Y2,
    /// 3 年。
    Y3,
    /// 5 年。
    Y5,
    /// 10 年。
    Y10,
}

impl CagrPeriod {
    /// 全部期間，依長度由短至長排列。
    pub const ALL: [CagrPeriod; 8] = [
        CagrPeriod::M3,
        CagrPeriod::M6,
        CagrPeriod::Y1,
        CagrPeriod::Y1H,
        CagrPeriod::Y2,
        CagrPeriod::Y3,
        CagrPeriod::Y5,
        CagrPeriod::Y10,
    ];

    /// 資料庫與 API 使用的期間代碼。
    pub fn code(&self) -> &'static str {
        match self {
            CagrPeriod::M3 => "M3",
            CagrPeriod::M6 => "M6",
            CagrPeriod::Y1 => "Y1",
            CagrPeriod::Y1H => "Y1H",
            CagrPeriod::Y2 => "Y2",
            CagrPeriod::Y3 => "Y3",
            CagrPeriod::Y5 => "Y5",
            CagrPeriod::Y10 => "Y10",
        }
    }

    /// 自期間代碼還原，無法辨識時回傳 `None`。
    pub fn from_code(code: &str) -> Option<CagrPeriod> {
        CagrPeriod::ALL.into_iter().find(|p| p.code() == code)
    }

    /// 期初目標日的回推月數。
    pub fn months(&self) -> u32 {
        match self {
            CagrPeriod::M3 => 3,
            CagrPeriod::M6 => 6,
            CagrPeriod::Y1 => 12,
            CagrPeriod::Y1H => 18,
            CagrPeriod::Y2 => 24,
            CagrPeriod::Y3 => 36,
            CagrPeriod::Y5 => 60,
            CagrPeriod::Y10 => 120,
        }
    }

    /// 顯示用的中文名稱。
    pub fn display_name(&self) -> &'static str {
        match self {
            CagrPeriod::M3 => "3 個月",
            CagrPeriod::M6 => "6 個月",
            CagrPeriod::Y1 => "1 年",
            CagrPeriod::Y1H => "1 年 6 個月",
            CagrPeriod::Y2 => "2 年",
            CagrPeriod::Y3 => "3 年",
            CagrPeriod::Y5 => "5 年",
            CagrPeriod::Y10 => "10 年",
        }
    }

    /// 年化數值是否具備參考意義。
    ///
    /// 3 個月的年化指數約為 4，單季 +20% 會被放大成 +107%，統計意義薄弱。
    /// 展示層應依此決定預設以「區間總報酬」或「年化報酬」排序。
    pub fn annualization_is_meaningful(&self) -> bool {
        self.months() >= 12
    }

    /// 是否適合作為預設曝露的期間。
    ///
    /// Y5／Y10 的樣本涵蓋率僅約 6 成（2026-08-07 實測 65.1%／59.0%），
    /// 可以提供但不宜預設，且展示時必須揭露樣本涵蓋率與存活者偏誤。
    pub fn is_default_exposed(&self) -> bool {
        self.months() <= 36
    }

    /// 是否需要在展示層揭露樣本涵蓋限制。
    pub fn requires_coverage_disclosure(&self) -> bool {
        !self.is_default_exposed()
    }

    /// 純價格報酬（口徑 A）在此期間是否仍具參考價值。
    ///
    /// 2026-08-07 實測顯示近十年每年皆有 134–216 檔股票配股，
    /// 期間愈長，忽略配股造成的低估愈嚴重，故長期間不提供此口徑。
    pub fn supports_price_metric(&self) -> bool {
        self.months() <= 36
    }
}

/// 報酬口徑。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CagrMetric {
    /// A：純價格報酬，忽略所有除權息。此口徑的 `cash_received` 恆為零。
    Price,
    /// B：含息不再投入 —— 配股自動入帳，現金股利累積不動。主指標。
    Total,
    /// C：含息再投入 —— 現金股利於除息日以當日收盤價買回。
    Reinvested,
}

impl CagrMetric {
    /// 全部口徑。
    pub const ALL: [CagrMetric; 3] = [CagrMetric::Price, CagrMetric::Total, CagrMetric::Reinvested];

    /// API 與查詢參數使用的代碼。
    pub fn code(&self) -> &'static str {
        match self {
            CagrMetric::Price => "price",
            CagrMetric::Total => "total",
            CagrMetric::Reinvested => "reinvested",
        }
    }

    /// 自代碼還原，無法辨識時回傳 `None`。
    pub fn from_code(code: &str) -> Option<CagrMetric> {
        CagrMetric::ALL.into_iter().find(|m| m.code() == code)
    }
}

/// 單一除權息事件。
///
/// 現金與股票股利各有獨立的除權息日，兩者可能不同日甚至只有其中之一，
/// 因此以兩個 `Option<NaiveDate>` 表達，由模擬器各自判定是否落在期間內。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DividendEvent {
    /// 股票代號。
    pub stock_symbol: String,
    /// 除息日（現金股利）。無效或未公布時為 `None`。
    pub ex_dividend_date_cash: Option<NaiveDate>,
    /// 除權日（股票股利）。無效或未公布時為 `None`。
    pub ex_dividend_date_stock: Option<NaiveDate>,
    /// 每股現金股利（元）。
    pub cash_dividend: Decimal,
    /// 每股股票股利（元，面額 [`PAR_VALUE`]）。
    pub stock_dividend: Decimal,
}

impl DividendEvent {
    /// 事件排序用的日期：取現金與股票除權息日中較早者。
    ///
    /// 兩者皆為 `None` 時回傳 `None`，此類事件不應納入模擬。
    pub fn sort_key(&self) -> Option<NaiveDate> {
        match (self.ex_dividend_date_cash, self.ex_dividend_date_stock) {
            (Some(cash), Some(stock)) => Some(cash.min(stock)),
            (Some(cash), None) => Some(cash),
            (None, Some(stock)) => Some(stock),
            (None, None) => None,
        }
    }
}

/// 單一口徑的模擬結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulationOutcome {
    /// 期末持有股數（含配股）。
    pub end_shares: Decimal,
    /// 累積現金股利（元）。
    ///
    /// 再投入口徑通常為零（股利都買回股數了），但除息日查無收盤價而無法買回
    /// 時，該次股利會退回現金累積 —— 不可丟棄 —— 此時不為零。
    pub cash_received: Decimal,
    /// 期末總價值（元）。
    pub end_value: Decimal,
    /// 區間總報酬率（%）。
    pub total_return_pct: Decimal,
    /// 年化報酬率（%）。
    pub cagr_pct: Decimal,
}

/// 單一股票在單一期間的 CAGR 計算結果 (Aggregate Root)。
///
/// 資料不足時 `data_complete` 為 `false`，且所有數值欄位為 `None` ——
/// 絕不以 0 代替，否則「上市半年的新股」會被誤讀為「十年零報酬」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockCagr {
    /// 計算基準日（期末交易日）。
    pub date: NaiveDate,
    /// 股票代號。
    pub stock_symbol: String,
    /// 統計期間。
    pub period: CagrPeriod,
    /// 實際採用的期初交易日。
    pub base_date: Option<NaiveDate>,
    /// 期初收盤價。
    pub base_price: Option<Decimal>,
    /// 期末收盤價。
    pub end_price: Option<Decimal>,
    /// 實際年數 =（期末交易日 − 期初交易日）/ 365。
    pub years: Option<Decimal>,
    /// 口徑 A：純價格報酬。
    pub price: Option<SimulationOutcome>,
    /// 口徑 B：含息不再投入。
    pub total: Option<SimulationOutcome>,
    /// 口徑 C：含息再投入。
    pub reinvested: Option<SimulationOutcome>,
    /// 該股在 `DailyQuotes` 的最早報價日，用於判定新上市／資料未涵蓋。
    pub first_quote_date: Option<NaiveDate>,
    /// `base_date` 較名目期初目標日晚幾天；0 表完全對齊。
    pub shortfall_days: Option<i32>,
    /// 期初資料是否齊全。
    pub data_complete: bool,
    /// 期間內偵測到無對應除權息的異常跳動（疑似減資或分割）。
    pub has_anomaly: bool,
    /// 期間內實際採計的除權息次數。
    pub dividend_events: i32,
}

/// 指定基準日與期間的樣本涵蓋統計。
///
/// 長期間（Y5／Y10）的可算比例僅約 6 成，展示層必須揭露此資訊，
/// 使用者才知道排行榜的分母是什麼、以及存活者偏誤的存在。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CagrCoverage {
    /// 母體檔數（未下市）。
    pub universe: i64,
    /// 實際可算檔數。
    pub counted: i64,
    /// 資料不足檔數。
    pub incomplete: i64,
    /// 標記為疑似異常的檔數。
    pub anomaly_flagged: i64,
    /// 可算檔數中報酬為正的檔數。
    pub positive: i64,
}

impl CagrCoverage {
    /// 樣本涵蓋率 = 可算檔數 / 母體檔數。母體為零時回傳零。
    pub fn coverage_ratio(&self) -> Decimal {
        if self.universe == 0 {
            return Decimal::ZERO;
        }
        Decimal::from(self.counted) / Decimal::from(self.universe)
    }

    /// 正報酬比例 = 正報酬檔數 / **可算檔數**。
    ///
    /// 分母刻意為 `counted` 而非 `universe` —— 混用會安靜地得到偏低的數字。
    pub fn positive_ratio(&self) -> Decimal {
        if self.counted == 0 {
            return Decimal::ZERO;
        }
        Decimal::from(self.positive) / Decimal::from(self.counted)
    }
}

impl crate::core::util::map::Keyable for StockCagr {
    fn key(&self) -> String {
        format!("{}-{}-{}", self.stock_symbol, self.date, self.period.code())
    }

    fn key_with_prefix(&self) -> String {
        format!(
            "StockCagr:{}-{}-{}",
            self.stock_symbol,
            self.date,
            self.period.code()
        )
    }
}
