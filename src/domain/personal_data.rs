//! # 個人業務資料表清單
//!
//! 會員系統計畫 4.0 第 ⑦ 區：資料屬於某位會員的既有業務表。會員本人的資料刪除、匯出與轉換家庭
//! （4.6、2.7 H1）要逐表處理這份清單。這些表除了 `trace` 之外，`member_id` 都沒有指向
//! `"Member"` 的外來鍵（`dividend_record_detail*` 經 `stock_ownership_details` 間接歸屬），
//! 所以不能靠外來鍵追溯。
//!
//! 新增含 `member_id` 的表時，必須登記在 [`PERSONAL_BUSINESS_TABLES`] 或 [`IAM_TABLES`]，
//! 並同步更新 `etc/sql/member_db_roles.sql` 的 `personal_tables`（`stock_readonly` 不得讀取
//! 個人業務表）；單元測試會掃描 `etc/sql/*.sql` 檢查。

/// 個人業務表（13 張），順序與計畫 4.0 第 ⑦ 區相同。
pub const PERSONAL_BUSINESS_TABLES: [&str; 13] = [
    "stock_ownership_details",
    "dividend_record_detail",
    "dividend_record_detail_more",
    "sell_history",
    "daily_money_history_member",
    "daily_money_history_detail",
    "daily_money_history_detail_more",
    "daily_money_history",
    "dividend_forecast_snapshot",
    "portfolio_target",
    "portfolio_target_dimension",
    "portfolio_target_revision",
    "trace",
];

/// 會員系統本身的表（帳號、群組、Session、Token、通知、配額等）。含 `member_id` 但不是業務資料，
/// 由 IAM 流程（stock_go）自行處理刪除與清理，不列入個人業務表。
pub const IAM_TABLES: [&str; 19] = [
    "Member",
    "member_household",
    "member_group",
    "member_group_permission",
    "member_group_mapping",
    "member_mcp_token",
    "member_audit_log",
    "notify_topic",
    "member_notify_channel",
    "member_notify_subscription",
    "member_notify_delivery",
    "member_session",
    "member_analysis_usage",
    "member_analysis_lease",
    "member_invitation",
    "member_email_challenge",
    "member_data_request",
    "member_mfa",
    "member_mfa_recovery_code",
];

#[cfg(test)]
mod tests;
