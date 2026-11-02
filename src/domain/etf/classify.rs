//! 由代號尾碼、名稱與來源提示（Yahoo 的類型、主動式標記、投資地區、追蹤指數）推得 ETF 的分類。
//!
//! 證交所的代號尾碼有固定意義：L 槓桿、R 反向、U 期貨、B 債券、A 主動式股票、D 主動式債券、
//! T 主動式平衡、K 外幣計價、C 債券的另一種計價。尾碼判斷不了的，再看名稱關鍵字。
//! 推斷難免有誤，`etf_profile.manual_override` 可以人工覆寫，覆寫在重建快照時套用。

/// 本站的 ETF 類型（篩選列的類型 chip）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// 市值型：追蹤大盤或市值加權指數。
    MarketCap,
    /// 高股息。
    HighDividend,
    /// 主動式股票、平衡型。
    Active,
    /// 債券（含主動式債券）。
    Bond,
    /// 槓桿。
    Leveraged,
    /// 反向。
    Inverse,
    /// 期貨、商品。
    Commodity,
    /// 海外股票。
    Overseas,
    /// 其他主題型。
    Thematic,
}

impl Strategy {
    /// 寫入資料庫與 API 的代碼。
    pub const fn code(self) -> &'static str {
        match self {
            Self::MarketCap => "market_cap",
            Self::HighDividend => "high_dividend",
            Self::Active => "active",
            Self::Bond => "bond",
            Self::Leveraged => "leveraged",
            Self::Inverse => "inverse",
            Self::Commodity => "commodity",
            Self::Overseas => "overseas",
            Self::Thematic => "thematic",
        }
    }
}

/// 來源提供的分類提示；沒有來源資料時全部留空。
#[derive(Debug, Clone, Default)]
pub struct ClassifyHint<'a> {
    /// 來源的類型（指數型、主動型…）。
    pub category: &'a str,
    /// 來源標示為主動式。
    pub is_active: bool,
    /// 來源的投資地區。
    pub region: &'a str,
    /// 追蹤指數。
    pub benchmark: &'a str,
}

/// 分類結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// 類型。
    pub strategy: Strategy,
    /// 投資地區；判斷不出來為空字串。
    pub region: String,
    /// 題材（依 [`THEMES`] 的順序）。
    pub themes: Vec<String>,
}

const HIGH_DIVIDEND_WORDS: &[&str] = &[
    "高股息",
    "高息",
    "優息",
    "股利",
    "收益",
    "存股",
    "收息",
    "填息",
];

const MARKET_CAP_WORDS: &[&str] = &[
    "台灣50",
    "臺灣50",
    "台50",
    "台灣加權",
    "臺灣加權",
    "中型100",
    "富櫃50",
    "MSCI台灣",
    "台灣卓越50",
    "TOP50",
    "發行量加權",
];

const COMMODITY_WORDS: &[&str] = &[
    "黃金",
    "石油",
    "原油",
    "黃豆",
    "白銀",
    "銅",
    "布蘭特",
    "玉米",
];

/// 題材與其名稱關鍵字，依篩選列下拉的順序。
const THEMES: &[(&str, &[&str])] = &[
    ("半導體", &["半導體", "晶片", "IC設計", "費城"]),
    ("人工智慧", &["AI", "人工智慧", "算力", "機器人", "智能"]),
    (
        "科技",
        &[
            "科技", "電子", "網通", "5G", "通訊", "雲端", "資安", "NASDAQ",
        ],
    ),
    ("ESG", &["ESG", "永續", "低碳", "綠能", "綠色", "潔淨"]),
    ("電動車", &["電動車", "電池", "未來車"]),
    ("金融", &["金融", "銀行"]),
    (
        "原物料",
        &[
            "黃金",
            "石油",
            "原油",
            "黃豆",
            "白銀",
            "銅",
            "布蘭特",
            "能源",
            "原物料",
        ],
    ),
];

/// 名稱關鍵字對應的投資地區，依序比對、先中先贏（「中信中國」要先比中國、不能落到台灣）。
const REGIONS: &[(&str, &[&str])] = &[
    (
        "美國",
        &[
            "美國",
            "美債",
            "S&P",
            "標普",
            "NASDAQ",
            "那斯達克",
            "費城",
            "道瓊",
            "北美",
        ],
    ),
    (
        "中國",
        &["中國", "滬深", "上證", "上証", "深証", "深100", "A50", "陸"],
    ),
    ("香港", &["香港", "恒生", "恆生"]),
    ("日本", &["日本", "日經", "東証", "東證"]),
    ("印度", &["印度"]),
    ("越南", &["越南"]),
    ("韓國", &["韓國"]),
    ("歐洲", &["歐洲", "歐元", "德國"]),
    ("全球", &["全球", "世界", "國際", "新興", "亞洲", "亞太"]),
];

/// 推得一檔 ETF 的類型、投資地區與題材。
pub fn classify(symbol: &str, name: &str, hint: &ClassifyHint<'_>) -> Classification {
    let suffix = symbol
        .chars()
        .last()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_uppercase());
    let region = normalize_region(hint.region)
        .or_else(|| region_from_name(name))
        .unwrap_or_else(|| default_region(suffix));
    let contains_any = |words: &[&str]| words.iter().any(|word| name.contains(word));

    let strategy = match suffix {
        Some('L') => Strategy::Leveraged,
        Some('R') => Strategy::Inverse,
        Some('U') => Strategy::Commodity,
        Some('B' | 'C' | 'D') => Strategy::Bond,
        _ if name.contains("正2") => Strategy::Leveraged,
        _ if name.contains("反1") => Strategy::Inverse,
        _ if name.starts_with('期') || contains_any(COMMODITY_WORDS) => Strategy::Commodity,
        _ if name.contains("債") => Strategy::Bond,
        Some('A' | 'T') => Strategy::Active,
        _ if hint.is_active || hint.category.contains("主動") || name.starts_with("主動") => {
            Strategy::Active
        }
        _ if contains_any(HIGH_DIVIDEND_WORDS) => Strategy::HighDividend,
        _ if region != "台灣" && !region.is_empty() => Strategy::Overseas,
        _ if contains_any(MARKET_CAP_WORDS) || is_market_cap_benchmark(hint.benchmark) => {
            Strategy::MarketCap
        }
        _ => Strategy::Thematic,
    };

    let mut themes: Vec<String> = THEMES
        .iter()
        .filter(|(_, words)| contains_any(words))
        .map(|(theme, _)| (*theme).to_string())
        .collect();
    if suffix == Some('K') {
        themes.push("外幣計價".to_string());
    }

    Classification {
        strategy,
        region,
        themes,
    }
}

/// 來源的配息頻率（Yahoo 的 `divCashFreq`）轉成本站代碼；不認得或沒有時為空字串。
pub fn dividend_frequency(source: &str) -> &'static str {
    match source.trim().to_ascii_lowercase().as_str() {
        "monthly" => "monthly",
        "quarterly" => "quarterly",
        "semi-annual" | "semiannual" | "semi_annual" => "semi_annual",
        "annual" | "annually" | "yearly" => "annual",
        _ => "",
    }
}

fn normalize_region(region: &str) -> Option<String> {
    match region.trim() {
        "" => None,
        "台灣" | "臺灣" => Some("台灣".to_string()),
        other => Some(other.to_string()),
    }
}

fn region_from_name(name: &str) -> Option<String> {
    REGIONS
        .iter()
        .find(|(_, words)| words.iter().any(|word| name.contains(word)))
        .map(|(region, _)| (*region).to_string())
}

/// 名稱看不出地區時：債券與期貨多半投資海外、無從推斷，留空；其餘視為台灣。
fn default_region(suffix: Option<char>) -> String {
    match suffix {
        Some('B' | 'C' | 'D' | 'U') => String::new(),
        _ => "台灣".to_string(),
    }
}

fn is_market_cap_benchmark(benchmark: &str) -> bool {
    [
        "臺灣50指數",
        "台灣50指數",
        "發行量加權股價指數",
        "臺灣中型100指數",
        "富櫃50指數",
        "MSCI臺灣指數",
        "MSCI台灣指數",
        "富時臺灣50",
    ]
    .iter()
    .any(|name| benchmark.contains(name))
}

#[cfg(test)]
mod tests;
