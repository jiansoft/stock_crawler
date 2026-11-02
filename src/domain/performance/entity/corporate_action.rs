//! 公司行動（股票分割、反向分割、減資）與其類型。

use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 公司行動的類型，對應資料表 `corporate_action.action_type` 的字串值。
///
/// 型別**不可**由 [`CorporateAction::share_ratio`] 反推。減資退還股款時，
/// 恢復買賣參考價同時扣掉了退還的現金，因此比例可能大於 1
/// （例如 8201 無敵 2016-07-18：前收 8.69、參考價 8.12，比例 1.0702），
/// 單看比例會把它誤判成分割。來源已知型別時一律明確指定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorporateActionType {
    /// 股票分割，含 ETF 的受益權單位分割。
    Split,
    /// 反向分割（股數變少、每股淨值等比例變高）。
    ReverseSplit,
    /// 減資，包含彌補虧損與退還股款兩種原因。
    CapitalReduction,
}

impl CorporateActionType {
    /// 轉成資料庫儲存的字串值。
    pub fn as_str(&self) -> &'static str {
        match self {
            CorporateActionType::Split => "split",
            CorporateActionType::ReverseSplit => "reverse_split",
            CorporateActionType::CapitalReduction => "capital_reduction",
        }
    }

    /// 自資料庫字串值還原，無法辨識時回傳 `None`。
    pub fn from_db_value(value: &str) -> Option<CorporateActionType> {
        match value {
            "split" => Some(CorporateActionType::Split),
            "reverse_split" => Some(CorporateActionType::ReverseSplit),
            "capital_reduction" => Some(CorporateActionType::CapitalReduction),
            _ => None,
        }
    }

    /// 僅憑比例推斷類型。
    ///
    /// **這是不得已時的退路**，只用在維運人員手動登錄（gRPC／backfill admin）
    /// 這種「除了比例之外沒有其他資訊」的情境。如上方型別說明，
    /// 減資退還股款的比例可能大於 1，此時本函式會回傳
    /// [`CorporateActionType::Split`] —— 那是已知且無法從比例本身修正的誤判。
    /// 凡是來源明確知道類型者（例如減資爬蟲）都不該呼叫這個函式。
    pub fn infer_from_ratio(share_ratio: Decimal) -> CorporateActionType {
        if share_ratio < Decimal::ONE {
            CorporateActionType::CapitalReduction
        } else {
            CorporateActionType::Split
        }
    }
}

/// 公司行動（股票分割、反向分割、減資）。
///
/// 本專案的報價一律是原始成交價，遇到這類事件價格會出現無法用除權息解釋的
/// 跳動。若不調整，2025-06-18 元大台灣50 的 1:4 分割會讓兩年期報酬看起來是
/// −38%，實際上該期間是上漲的。
///
/// 上市減資可由 [`crate::infra::crawler::twse::capital_reduction`] 全量取得；
/// 分割與 ETF 受益權單位分割不在任何公開端點內，仍由人工維護。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorporateAction {
    /// 股票代號。
    pub stock_symbol: String,
    /// 生效日：換發後恢復交易的第一個交易日（該日收盤價已是調整後價格）。
    pub effective_date: NaiveDate,
    /// 事件類型。不由 [`Self::share_ratio`] 反推，理由見 [`CorporateActionType`]。
    pub action_type: CorporateActionType,
    /// 股數變動比例：持有 1 股在事件後變成幾股。
    ///
    /// 1 股分割成 4 股為 `4`；減資三成（1,000 股變 700 股）為 `0.7`。
    /// 價格的變動方向與此相反，模擬時只需調整股數，價格用當日的真實報價即可。
    pub share_ratio: Decimal,
    /// 備註（例如「1:4 分割」、「減資彌補虧損」）。
    pub note: String,
}

impl CorporateAction {
    /// 由生效前最後收盤價還原生效日（恢復買賣日）的參考價：`前收 ÷ share_ratio`。
    ///
    /// 減資的比例本來就是交易所「停止買賣前收盤價 ÷ 恢復買賣參考價」，退還股款也一樣
    /// （見 [`CorporateActionType`]），反除即回到參考價；分割、反向分割同理。
    /// 前收或比例不為正時回傳 `None`。
    ///
    /// 只用於「成交價是否偏離合理區間」的檢查，不處理升降單位進位，精度取小數兩位。
    pub fn reference_price(&self, previous_close: Decimal) -> Option<Decimal> {
        if previous_close <= Decimal::ZERO || self.share_ratio <= Decimal::ZERO {
            return None;
        }

        previous_close
            .checked_div(self.share_ratio)
            .map(|price| price.round_dp(2))
    }
}
