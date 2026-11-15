//! `/portfolio/targets/revisions` 的 request/response DTO 與 OpenAPI schema：目標配置的歷史版本與還原。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::PortfolioTargetItemDto;

/// 查詢目標配置歷史版本的 Query 參數。
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct PortfolioTargetRevisionsQueryParams {
    /// 會員編號；未帶或 0 為所有部位（預設 0）。
    #[param(default = 0, minimum = 0)]
    pub member_id: Option<i64>,
    /// 家庭編號；未帶時為家庭 #1（會員系統上線前只有這個家庭）。配置目標依家庭分開，
    /// `member_id = 0` 是該家庭的全家目標。
    #[param(minimum = 1)]
    pub household_id: Option<i64>,
    /// 配置維度：`stock`、`industry`、`etf_look_through`（必填）。
    pub dimension: String,
}

/// 目標配置的一個歷史版本：某次儲存前的整組配置。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct PortfolioTargetRevisionDto {
    /// 版本編號（還原時使用）。
    pub id: i64,
    /// 這一組被取代（記成這一版）的時間（RFC 3339）。
    pub saved_time: String,
    /// 那一組的目標配置清單，依比例由高到低；空的代表全現金。
    pub targets: Vec<PortfolioTargetItemDto>,
    /// 那一組的現金比例（100% − 各標的目標權重總和）。
    pub target_cash_percent: f64,
    /// 取代這一組的那次寫入來源：`web`、`mcp`；舊資料為空字串。
    pub actor_source: String,
    /// 取代這一組的那次寫入參照（web 為操作會員編號、mcp 為 Token id）；舊資料為空字串。
    pub actor_ref: String,
}

/// 目標配置歷史版本清單回應。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct PortfolioTargetRevisionsResponse {
    /// 家庭編號。
    pub household_id: i64,
    /// 會員編號。
    pub member_id: i64,
    /// 配置維度。
    pub dimension: String,
    /// 歷史版本，由新到舊（每個維度最多保留 20 版）。
    pub revisions: Vec<PortfolioTargetRevisionDto>,
    /// 免責聲明文字。
    pub disclaimer: &'static str,
}

/// 還原目標配置版本的 Request Body。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RestorePortfolioTargetRevisionRequest {
    /// 會員編號；未帶或 0 為所有部位（預設 0），必須是該版本所屬的會員。
    #[schema(default = 0, minimum = 0)]
    pub member_id: Option<i64>,
    /// 家庭編號；未帶時為家庭 #1（會員系統上線前只有這個家庭）。配置目標依家庭分開，
    /// `member_id = 0` 是該家庭的全家目標。
    #[schema(minimum = 1)]
    pub household_id: Option<i64>,
    /// 寫入來源：`web`（網站）或 `mcp`（stock-mcp-go）；未帶為未標示。記在被取代的那一版上。
    pub actor_source: Option<String>,
    /// 來源內的參照：web 為操作會員編號、mcp 為 Token id（最多 64 字元）。
    pub actor_ref: Option<String>,
}
