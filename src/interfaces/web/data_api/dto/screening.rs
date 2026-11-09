//! `/stocks/screen` 條件選股的 request、response 與 OpenAPI schema。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// OpenAPI 文件使用的四種估值分類。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum ValuationBandParamValue {
    /// 不高於便宜價。
    Undervalued,
    /// 便宜價至合理價。
    FairValued,
    /// 合理價至昂貴價。
    Overvalued,
    /// 高於昂貴價。
    HighlyOvervalued,
}

/// OpenAPI 文件使用的選股排序欄位。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum StockScreenSortValue {
    /// 股票代號。
    StockSymbol,
    /// 營收年增率。
    RevenueYoy,
    /// 連續營收年增月數。
    RevenueYoyStreak,
    /// 近 3 個月累計營收年增率。
    #[schema(rename = "revenue_yoy_3m")]
    RevenueYoy3m,
    /// 最新月營收。
    MonthlyRevenue,
    /// 每股盈餘。
    Eps,
    /// 股東權益報酬率。
    Roe,
    /// 殖利率。
    DividendYield,
    /// 估值百分比。
    ValuationPercentage,
    /// Piotroski F 分數。
    FScore,
    /// 最新一季毛利率。
    GrossMargin,
    /// 最新一季負債比。
    DebtRatio,
    /// 最新一季營業利益率。
    OperatingMargin,
    /// 相對強度分數。
    RsScore,
    /// 本益比。
    PeRatio,
    /// 股價淨值比。
    PbRatio,
    /// 最新一季單季 EPS 對去年同季的成長率。
    EpsYoy,
    /// 近 3 個年度 EPS 年增率中最低的一年。
    AnnualEpsGrowth,
}

/// OpenAPI 文件使用的排序方向。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum SortOrderParamValue {
    /// 升冪。
    Asc,
    /// 降冪。
    Desc,
}

/// 條件選股結果中的單一股票。
///
/// 各來源期間即使已超過新鮮度上限仍會保留，方便呼叫端判斷資料為何被
/// 轉成 `null`；過期的指標本身不參與篩選，也不會被誤當成目前數值。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenedStock {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場編號（上市 2、上櫃 4）。
    pub(crate) market_id: i32,
    /// 產業分類編號。
    pub(crate) industry_id: i32,
    /// 最新且仍在三個月內的營收年增率；缺值或過期時為 `null`。
    pub(crate) revenue_yoy_percent: Option<f64>,
    /// 從最新月份往回連續年增（年增率 > 0 且月份相連）的月數，最多算 12 個月；
    /// 最新營收過期時為 `null`。
    pub(crate) revenue_yoy_streak: Option<u8>,
    /// 營收加速：從最新月份往回，年增率逐月高於前一個月（月份相連）的月數，最多 11 個月；
    /// 最新營收過期時為 `null`。
    pub(crate) revenue_acceleration_months: Option<u8>,
    /// 近 3 個月合計營收對去年同期的年增率百分比；三個月不相連、去年同期為 0
    /// 或最新營收過期時為 `null`。
    pub(crate) revenue_yoy_3m_percent: Option<f64>,
    /// 最新一個月營收（新台幣千元）；最新營收過期時為 `null`。
    pub(crate) monthly_revenue: Option<f64>,
    /// 最新且仍在兩季內的季度 EPS；缺值或過期時為 `null`。
    pub(crate) earnings_per_share: Option<f64>,
    /// 最新且仍在兩季內的季度 ROE；缺值或過期時為 `null`。
    pub(crate) return_on_equity: Option<f64>,
    /// 最新且仍在 31 天內的殖利率百分比；缺值或過期時為 `null`。
    pub(crate) dividend_yield_percent: Option<f64>,
    /// 最新且仍在 31 天內的估值區間；缺值或過期時為 `null`。
    pub(crate) valuation_band: Option<String>,
    /// 最新且仍在 31 天內的估值百分比；缺值或過期時為 `null`。
    pub(crate) valuation_percentage: Option<f64>,
    /// 該股票最新營收月份，格式 `YYYY-MM`；過期時仍保留。
    pub(crate) revenue_month: Option<String>,
    /// 該股票最新季度財報期間，格式 `YYYY-Q1`～`YYYY-Q4`；過期時仍保留。
    pub(crate) financial_period: Option<String>,
    /// 該股票最新估值日期，格式 `YYYY-MM-DD`；過期時仍保留。
    pub(crate) valuation_date: Option<String>,
    /// 該股票最新殖利率日期，格式 `YYYY-MM-DD`；過期時仍保留。
    pub(crate) yield_date: Option<String>,
    /// Piotroski F 分數（0–9）；評分年度落後查詢日超過 2 年或沒有年報時為 `null`。
    pub(crate) f_score: Option<u8>,
    /// F 分數可判定的指標數；與 `f_score` 同時為 `null`。
    pub(crate) f_score_evaluated: Option<u8>,
    /// F 分數的評分年度；過期時仍保留。
    pub(crate) f_score_year: Option<i32>,
    /// 最新且仍在兩季內的單季毛利率百分比；缺值或過期時為 `null`。
    pub(crate) gross_margin_percent: Option<f64>,
    /// 最新且仍在兩季內的季末負債比百分比；缺值或過期時為 `null`。
    pub(crate) debt_ratio_percent: Option<f64>,
    /// 截至最新一季的近四季自由現金流合計（新台幣千元）；四季不齊或過期時為 `null`。
    pub(crate) free_cash_flow_ttm: Option<i64>,
    /// 毛利率、負債比與自由現金流的期間，格式 `YYYY-Q1`～`YYYY-Q4`；過期時仍保留。
    pub(crate) health_period: Option<String>,
    /// 最新且仍在兩季內的單季營業利益率百分比（Yahoo 單季損益表）；營收為 0、缺值或過期時為 `null`。
    pub(crate) operating_margin_percent: Option<f64>,
    /// 最新且仍在兩季內的單季稅後淨利率百分比；規則同 `operating_margin_percent`。
    pub(crate) net_margin_percent: Option<f64>,
    /// 三率三升：最新一季毛利率、營業利益率、稅後淨利率是否都高於去年同一季；去年同季沒有資料或
    /// 過期時為 `null`。
    pub(crate) margins_yoy_up: Option<bool>,
    /// 最新月營收是否創歷史新高（高於之前所有月份）；之前不滿 12 個月、沒有營收或過期時為 `null`。
    pub(crate) revenue_record: Option<bool>,
    /// 營業利益率、稅後淨利率與三率三升的損益表期間，格式 `YYYY-Q1`～`YYYY-Q4`；過期時仍保留。
    pub(crate) margin_period: Option<String>,
    /// 相對強度分數 1–99（同 `/market/relative-strength`，資料日見 `rs_as_of`）；
    /// 上市未滿 240 個交易日、暫停交易或尚無報酬資料時為 `null`。
    pub(crate) rs_score: Option<u8>,
    /// 截至 `chip_as_of` 的外資連續買賣超天數：正數為連續買超、負數為連續賣超、0 為當天沒有買賣；
    /// 從 2019 年籌碼資料起算，不限回看長度。當天沒有籌碼資料時為 `null`。
    pub(crate) foreign_streak: Option<i16>,
    /// 截至 `chip_as_of` 的投信連續買賣超天數，規則同 `foreign_streak`。
    pub(crate) trust_streak: Option<i16>,
    /// 查詢日（含）之前 45 天內最近 20 筆日線的平均成交金額（百萬元）；沒有日線時為 `null`。
    pub(crate) avg_trade_value_20d: Option<f64>,
    /// 查詢日（含）之前 10 天內最新一筆日線的官方本益比（證交所／櫃買中心）；虧損或沒有資料時為 `null`。
    pub(crate) pe_ratio: Option<f64>,
    /// 同一筆日線的官方股價淨值比；沒有資料時為 `null`。
    pub(crate) pb_ratio: Option<f64>,
    /// `pe_ratio`、`pb_ratio` 的日線日期，格式 `YYYY-MM-DD`；10 天內沒有日線時為 `null`。
    pub(crate) ratio_date: Option<String>,
    /// 設 `range_breakout_days` 時：最近一次區間突破的交易日，格式 `YYYY-MM-DD`；沒設時為 `null`。
    pub(crate) range_breakout_date: Option<String>,
    /// 設 `range_breakout_days` 時：最近一次區間突破的說明（突破價、整理幅度、量比、距突破價）；
    /// 沒設時為 `null`。
    pub(crate) range_breakout_description: Option<String>,
    /// 最新一季單季 EPS 對去年同季的成長率百分比（CANSLIM 的 C）；去年同季 EPS ≤ 0、缺值或財報過期時為 `null`。
    pub(crate) eps_yoy_percent: Option<f64>,
    /// 近 3 個年度 EPS 年增率中最低的一年（CANSLIM 的 A，「每年都至少成長多少」）；年度不滿 4 年、不連續、
    /// 任一基期 EPS ≤ 0，或最新年度落後查詢年超過 2 年時為 `null`。
    pub(crate) annual_eps_growth_percent: Option<f64>,
    /// `annual_eps_growth_percent` 比較的最新年度（西元）；沒有年度 EPS 時為 `null`。
    pub(crate) annual_eps_year: Option<i32>,
    /// 設 `new_high_days` 時：查詢日（含）以前一年內最近一次創 52 週新高的交易日，格式 `YYYY-MM-DD`；沒設時為 `null`。
    pub(crate) new_high_date: Option<String>,
    /// 近 5 年填息率（%）：除息日在查詢日前 5 年內、已觀察滿 120 個交易日的現金除息中，120 個交易日內收盤回到
    /// 填息價的比例；沒有觀察期已滿的除息時為 `null`。
    pub(crate) fill_rate_percent: Option<f64>,
    /// 同一批事件中已填息者的平均填息交易日數；都沒有填息時為 `null`。
    pub(crate) avg_fill_days: Option<f64>,
    /// 連續配發現金股利的年數（以除息日年度計，從最近一次除息往回數）；最近一次除息早於查詢年前一年時為 0。
    pub(crate) dividend_years: Option<i32>,
    /// 股利成長年數：以除息日年度加總現金股利，只看查詢年以前的完整年度；從查詢年前一年往回，每年都不低於
    /// 前一年（年度相連）的年數。查詢年前一年沒有配息時為 0。
    pub(crate) dividend_growth_years: Option<i32>,
}

/// 條件選股成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StockScreeningResponse {
    /// 混合資料來源沒有單一正確日期，因此固定為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// `rs_score` 的資料日（最近一個已算好報酬的交易日）；查無相對強度資料時為 `null`。
    pub(crate) rs_as_of: Option<String>,
    /// `foreign_streak`、`trust_streak` 的資料日（最新一個有籌碼資料的交易日，每晚 21:50 入庫）；
    /// 查無籌碼資料時為 `null`。
    pub(crate) chip_as_of: Option<String>,
    /// 通過所有固定條件的股票，最多五十筆。
    pub(crate) stocks: Vec<ScreenedStock>,
}

/// 條件選股 endpoint 的固定白名單 query string。
///
/// 所有數值欄位先在 handler 驗證範圍，再轉成 PostgreSQL `NUMERIC` 綁定值；
/// `sort_by` 與 `sort_order` 只會映射到程式內建的三十六個排序分支。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct StockScreeningParams {
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 可選的正整數產業分類編號。
    #[param(minimum = 1)]
    pub(crate) industry_id: Option<i32>,
    /// 排除的產業 id（逗號分隔，最多 10 個，例如 `17` 排除金融保險業，其營收口徑與一般公司不同、
    /// 毛利率與營業利益率排名常落在最前面）；不可包含 `industry_id`。單獨設定也算篩選條件。
    pub(crate) exclude_industry_ids: Option<String>,
    /// 估值區間固定 enum。
    #[param(value_type = ValuationBandParamValue, inline)]
    pub(crate) valuation_band: Option<String>,
    /// 最低營收年增率百分比，範圍 -100–10000。
    #[param(minimum = -100, maximum = 10000)]
    pub(crate) min_revenue_yoy_percent: Option<f64>,
    /// 最少連續營收年增月數，範圍 1–12。
    #[param(minimum = 1, maximum = 12)]
    pub(crate) min_revenue_yoy_streak: Option<u8>,
    /// 最低近 3 個月累計營收年增率百分比，範圍 -100–10000。
    #[param(minimum = -100, maximum = 10000)]
    pub(crate) min_revenue_yoy_3m_percent: Option<f64>,
    /// 最新一個月營收下限（新台幣千元），範圍 0–1000000000000；用來排除營收規模太小、
    /// 年增率因低基期而失真的股票。
    #[param(minimum = 0, maximum = 1000000000000_i64)]
    pub(crate) min_monthly_revenue: Option<f64>,
    /// 最低每股盈餘，範圍 -10000–10000。
    #[param(minimum = -10000, maximum = 10000)]
    pub(crate) min_eps: Option<f64>,
    /// 最低股東權益報酬率百分比，範圍 -10000–10000。
    #[param(minimum = -10000, maximum = 10000)]
    pub(crate) min_roe_percent: Option<f64>,
    /// 最低殖利率百分比，範圍 0–1000。
    #[param(minimum = 0, maximum = 1000)]
    pub(crate) min_dividend_yield_percent: Option<f64>,
    /// 最低 Piotroski F 分數，範圍 0–9。
    #[param(minimum = 0, maximum = 9)]
    pub(crate) min_f_score: Option<u8>,
    /// 最低單季毛利率百分比，範圍 -10000–100。
    #[param(minimum = -10000, maximum = 100)]
    pub(crate) min_gross_margin_percent: Option<f64>,
    /// 最高季末負債比百分比，範圍 0–1000。
    #[param(minimum = 0, maximum = 1000)]
    pub(crate) max_debt_ratio_percent: Option<f64>,
    /// 最低單季營業利益率百分比（Yahoo 最新一季單季損益表），範圍 -10000–100。
    #[param(minimum = -10000, maximum = 100)]
    pub(crate) min_operating_margin_percent: Option<f64>,
    /// `true` 時只留「三率三升」：最新一季毛利率、營業利益率、稅後淨利率都高於去年同一季；
    /// `false` 或省略不篩選。
    pub(crate) margins_yoy_up: Option<bool>,
    /// `true` 時只留最新月營收創歷史新高（高於之前所有月份，之前至少 12 個月）的股票；`false` 或省略
    /// 不篩選。
    pub(crate) revenue_record: Option<bool>,
    /// `true` 時只留近四季自由現金流為正的股票；`false` 或省略不篩選。
    pub(crate) positive_free_cash_flow: Option<bool>,
    /// 最低相對強度分數，範圍 1–99；分數以上市櫃普通股（不含 ETF、特別股）計算（同 `/market/relative-strength`），
    /// 沒有分數的股票不符合。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) min_rs_score: Option<u8>,
    /// 外資至少連續買超幾個交易日（截至最新籌碼資料日），範圍 1–60。
    #[param(minimum = 1, maximum = 60)]
    pub(crate) min_foreign_streak: Option<u8>,
    /// 投信至少連續買超幾個交易日（截至最新籌碼資料日），範圍 1–60。
    #[param(minimum = 1, maximum = 60)]
    pub(crate) min_trust_streak: Option<u8>,
    /// 近 20 筆日線平均成交金額下限（百萬元，1–100000）；例如 50 代表日均成交 5,000 萬元。
    /// 與條件選股回測的 `min_avg_trade_value` 相同，用來排除實際買不到量的股票。
    #[param(minimum = 1, maximum = 100000)]
    pub(crate) min_avg_trade_value: Option<u32>,
    /// 本益比上限（倍，0–10000）：取查詢日之前 10 天內最新一筆日線的官方本益比，虧損或沒有資料的股票
    /// 不符合。
    #[param(minimum = 0, maximum = 10000)]
    pub(crate) max_pe: Option<f64>,
    /// 股價淨值比上限（倍，0–1000），資料規則同 `max_pe`。
    #[param(minimum = 0, maximum = 1000)]
    pub(crate) max_pb: Option<f64>,
    /// 集保千張大戶持股比例最近一週相對 `major_change_weeks` 週前的變化下限（百分點，-100–100，可為負）。
    /// 只用日期早於查詢日的週（與回測相同：集保週五的資料週六才公布）；最近一週距查詢日超過 14 天、或兩週
    /// 任一週沒有該股資料的股票不列入。
    #[param(minimum = -100, maximum = 100)]
    pub(crate) min_major_change: Option<f64>,
    /// `min_major_change` 的回看週數（集保資料週，1–12，預設 4）。
    #[param(minimum = 1, maximum = 12, default = 4)]
    pub(crate) major_change_weeks: Option<u8>,
    /// 只留最近 N 個交易日（含查詢日，1–20）出現區間突破（`range_breakout`／`breakout`）、且突破後還沒
    /// 跌回突破價（沒有 `failed`）的股票；回應附最近一次突破的日期與說明。2019 年起回測單獨依區間突破
    /// 選股明顯輸 0050，只供參考。
    #[param(minimum = 1, maximum = 20)]
    pub(crate) range_breakout_days: Option<u8>,
    /// 最新一季單季 EPS 對去年同季的成長率下限（%，-100–10000；CANSLIM 的 C 建議 25）。去年同季 EPS ≤ 0
    /// 無法計算成長率，這類股票不符合；財報新鮮度同 `min_eps`（兩季內）。
    #[param(minimum = -100, maximum = 10000)]
    pub(crate) min_eps_yoy_percent: Option<f64>,
    /// 近 3 個年度 EPS「每年」年增率的下限（%，-100–10000；CANSLIM 的 A 建議 25）：取最近 4 個年度 EPS 算出
    /// 3 個年增率，以最低者比較。年度不滿 4 年、不連續、任一基期 ≤ 0，或最新年度落後查詢年超過 2 年都不符合。
    #[param(minimum = -100, maximum = 10000)]
    pub(crate) min_annual_eps_growth_percent: Option<f64>,
    /// 只留最近 N 個日曆天（含查詢日，1–365）內創過 52 週新高（`52_week`／`new_high` 訊號）的股票
    /// （CANSLIM 的 N）；回應附最近一次新高日。
    #[param(minimum = 1, maximum = 365)]
    pub(crate) new_high_days: Option<u16>,
    /// 近 5 年填息率下限（%，0–100）：除息日在查詢日前 5 年內、已觀察滿 120 個交易日的現金除息中，120 個交易日內
    /// 收盤回到填息價的比例；沒有觀察期已滿的除息不符合。
    #[param(minimum = 0, maximum = 100)]
    pub(crate) min_fill_rate_percent: Option<f64>,
    /// 近 5 年已填息者的平均填息交易日數上限（1–120）；都沒有填息的不符合。
    #[param(minimum = 1, maximum = 120)]
    pub(crate) max_avg_fill_days: Option<f64>,
    /// 連續配發現金股利的年數下限（1–30），以除息日年度計；最近一次除息早於查詢年前一年的不符合。
    #[param(minimum = 1, maximum = 30)]
    pub(crate) min_dividend_years: Option<u8>,
    /// 股利成長年數下限（1–30）：只看查詢年以前的完整年度，從查詢年前一年往回、年度現金股利都不低於前一年
    /// （年度相連）的年數；例如 5 代表連續 6 年配息且 5 次都沒減少。
    #[param(minimum = 1, maximum = 30)]
    pub(crate) min_dividend_growth_years: Option<u8>,
    /// 營收加速月數下限（1–11）：從最新一個月往回，年增率逐月高於前一個月（月份相連）的月數；例如 3 代表最近
    /// 3 個月的年增率一個比一個高。可搭配 `min_revenue_yoy_percent` 排除年增率仍為負的股票。
    #[param(minimum = 1, maximum = 11)]
    pub(crate) min_revenue_acceleration_months: Option<u8>,
    /// 排序欄位固定 enum；預設 `stock_symbol`。
    #[param(value_type = StockScreenSortValue, default = "stock_symbol")]
    pub(crate) sort_by: Option<String>,
    /// 排序方向：`asc`（預設）或 `desc`。
    #[param(value_type = SortOrderParamValue, default = "asc")]
    pub(crate) sort_order: Option<String>,
    /// 最多回傳筆數，預設 20，範圍 1–50。
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub(crate) limit: Option<u8>,
}
