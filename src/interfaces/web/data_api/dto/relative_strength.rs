//! `/market/relative-strength` 相對強度排行、`/stocks/{symbol}/relative-strength` 個股相對強度
//! 歷史與 `/market/industry-strength` 產業相對強度的 request、response 與 OpenAPI schema。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{MarketParamValue, SortOrderParamValue};

/// 相對強度排行的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct RelativeStrengthParams {
    /// 交易日（`YYYY-MM-DD`）；省略為最新一個已算好報酬的交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。只篩選輸出，分數與名次仍以上市櫃普通股（不含 ETF、特別股）計算。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 只列出這個產業（產業名稱，例如「半導體業」）；省略為全部。只篩選輸出，不影響分數。
    pub(crate) industry: Option<String>,
    /// 只列出這檔股票（查單一股票的分數與名次）。
    pub(crate) symbol: Option<String>,
    /// `desc`（預設，最強在前）或 `asc`（最弱在前）。
    #[param(value_type = SortOrderParamValue, default = "desc")]
    pub(crate) order: Option<String>,
    /// 最多回傳幾筆，預設 50，範圍 1–200。
    #[param(minimum = 1, maximum = 200, default = 50)]
    pub(crate) limit: Option<u8>,
}

/// 相對強度排行中的單一股票。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct RelativeStrengthItem {
    /// 在上市櫃普通股（不含 ETF、特別股）中的名次（1 為最強；加權報酬相同時名次相同）。
    pub(crate) rank: usize,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場：`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 產業名稱；未分類時為空字串。
    pub(crate) industry: String,
    /// 相對強度分數 1–99：加權報酬勝過全體的百分位（99 為最強）。
    pub(crate) rs_score: u8,
    /// 加權報酬（%）＝ 0.4 × 近 60 日 + 0.3 × 近 120 日 + 0.3 × 近 240 日。
    pub(crate) weighted_return: f64,
    /// 近 20 個交易日的還原報酬（%）；資料不足為 `null`。
    pub(crate) return_20d: Option<f64>,
    /// 近 60 個交易日的還原報酬（%）。
    pub(crate) return_60d: f64,
    /// 近 120 個交易日的還原報酬（%）。
    pub(crate) return_120d: f64,
    /// 近 240 個交易日的還原報酬（%）。
    pub(crate) return_240d: f64,
    /// 資料日收盤價（元）；當天沒有成交為 `null`。
    pub(crate) closing_price: Option<f64>,
}

/// 相對強度排行結果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct RelativeStrengthResponse {
    /// 資料日（各天期報酬的結束日）。
    pub(crate) data_as_of: String,
    /// 查詢的市場。
    pub(crate) market: String,
    /// 排序方向：`desc` 或 `asc`。
    pub(crate) order: String,
    /// 參與計分的股票數（上市櫃、近 60／120／240 日報酬都有的股票）。
    pub(crate) universe: usize,
    /// 篩選後的筆數（`limit` 截斷前）。
    pub(crate) matched: usize,
    /// 依 `order` 排序的前 `limit` 筆。
    pub(crate) items: Vec<RelativeStrengthItem>,
}

/// 個股相對強度歷史的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct RelativeStrengthHistoryParams {
    /// 最後一個交易日（`YYYY-MM-DD`）；省略為今天，遇休市日或尚未算好的交易日自動略過。
    pub(crate) date: Option<String>,
    /// 往回幾個交易日，預設 60，範圍 1–120。
    #[param(minimum = 1, maximum = 120, default = 60)]
    pub(crate) days: Option<u16>,
}

/// 單一交易日的相對強度。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct RelativeStrengthPoint {
    /// 交易日。
    pub(crate) date: String,
    /// 相對強度分數 1–99（與 `/market/relative-strength` 同一算法）。
    pub(crate) rs_score: u8,
    /// 當天在上市櫃普通股（不含 ETF、特別股）中的名次。
    pub(crate) rank: i64,
    /// 當天參與計分的股票數。
    pub(crate) universe: i64,
    /// 加權報酬（%）＝ 0.4 × 近 60 日 + 0.3 × 近 120 日 + 0.3 × 近 240 日。
    pub(crate) weighted_return: f64,
}

/// 相對強度分數穿越門檻的轉折。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct RelativeStrengthCrossing {
    /// 穿越當天。
    pub(crate) date: String,
    /// 門檻：80（強勢）或 50（中位）。
    pub(crate) threshold: u8,
    /// `up`：前一個交易日低於門檻、當天達到門檻；`down`：前一個交易日達到門檻、當天低於門檻。
    pub(crate) direction: String,
    /// 前一個交易日的分數。
    pub(crate) from_score: u8,
    /// 當天的分數。
    pub(crate) to_score: u8,
}

/// 個股相對強度歷史。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct RelativeStrengthHistoryResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 最新一筆的交易日；期間內未計分（上市未滿 240 個交易日、近 5 個交易日沒有收盤或非上市櫃）為
    /// `null`。
    pub(crate) data_as_of: Option<String>,
    /// 查詢的交易日數。
    pub(crate) days: u16,
    /// 最新分數。
    pub(crate) latest_score: Option<u8>,
    /// 期間內最高分數。
    pub(crate) high_score: Option<u8>,
    /// 期間內最高分數最近一次出現的交易日。
    pub(crate) high_date: Option<String>,
    /// 期間內最低分數。
    pub(crate) low_score: Option<u8>,
    /// 期間內最低分數最近一次出現的交易日。
    pub(crate) low_date: Option<String>,
    /// 最新分數減期間第一筆分數。
    pub(crate) score_change: Option<i16>,
    /// 期間內穿越 50／80 門檻的轉折，由新到舊。
    pub(crate) crossings: Vec<RelativeStrengthCrossing>,
    /// 每個交易日的分數，由新到舊。
    pub(crate) items: Vec<RelativeStrengthPoint>,
}

/// 產業相對強度的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndustryStrengthParams {
    /// 交易日（`YYYY-MM-DD`）；省略為最新一個已算好報酬的交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。只決定各產業納入哪些股票，分數仍以上市櫃普通股（不含 ETF、特別股）計算。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 產業至少要有幾檔計分股票才列出，預設 3，範圍 1–50（檔數太少的產業中位數不穩定）。
    #[param(minimum = 1, maximum = 50, default = 3)]
    pub(crate) min_stocks: Option<u8>,
    /// `desc`（預設，最強產業在前）或 `asc`（最弱產業在前），依 RS 中位數排序。
    #[param(value_type = SortOrderParamValue, default = "desc")]
    pub(crate) order: Option<String>,
}

/// 產業內相對強度最高的個股。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct IndustryLeader {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 相對強度分數 1–99。
    pub(crate) rs_score: u8,
}

/// 單一產業的相對強度。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct IndustryStrengthItem {
    /// 依 RS 中位數由強到弱的名次（同中位數同名次）。
    pub(crate) rank: usize,
    /// 產業名稱。
    pub(crate) industry: String,
    /// 計分股票數。
    pub(crate) stocks: usize,
    /// 成分股 RS 分數中位數。
    pub(crate) median_rs_score: f64,
    /// RS ≥ 80 的檔數占比（0–1）。
    pub(crate) strong_ratio: f64,
    /// RS ≤ 20 的檔數占比（0–1）。
    pub(crate) weak_ratio: f64,
    /// 成分股加權報酬（%）的平均。
    pub(crate) avg_weighted_return: f64,
    /// 約 20 個交易日前（見 `prior_as_of`）的 RS 中位數；當時沒有計分股票為 `null`。
    pub(crate) prior_median_rs_score: Option<f64>,
    /// RS 中位數的變化（本期減約 20 個交易日前）。
    pub(crate) median_change: Option<f64>,
    /// RS 最高的前三檔。
    pub(crate) leaders: Vec<IndustryLeader>,
}

/// 產業相對強度結果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct IndustryStrengthResponse {
    /// 資料日。
    pub(crate) data_as_of: String,
    /// 比較基準日（資料日往前約 20 個交易日）；查無當時資料為 `null`。
    pub(crate) prior_as_of: Option<String>,
    /// 查詢的市場。
    pub(crate) market: String,
    /// 排序方向：`desc` 或 `asc`。
    pub(crate) order: String,
    /// 參與計分的上市櫃股票數。
    pub(crate) universe: usize,
    /// 依 RS 中位數排序的產業。
    pub(crate) items: Vec<IndustryStrengthItem>,
}
