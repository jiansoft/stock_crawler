//! 版本化唯讀股票 Data API 與其線上 OpenAPI 文件。
//!
//! 此模組把 PostgreSQL 的內部 schema 隔離於 HTTP 契約之外；MCP 等內網服務
//! 只需依 `/api-docs/openapi.json` 建立 client，並可從 `/swagger-ui` 互動測試。
//!
//! 檔案分工：`routes` 負責路由註冊與 Bearer 保護範圍、`openapi` 負責 OpenAPI
//! 文件定義、`handlers` 是各 endpoint 的實作、`dto` 是 HTTP 契約型別、
//! `auth` 是市場類端點的 Bearer token 驗證 middleware、`scope` 是持股類端點
//! （`/portfolio/*`）的金鑰與資料範圍 middleware。

mod auth;
mod budget;
mod dto;
mod handlers;
mod openapi;
mod routes;
mod scope;

pub use handlers::BacktestStrategyGateway;
pub(crate) use handlers::validate_strategy_query;
pub(super) use routes::router;

/// 啟動檢查：開放註冊時持股類端點必須是嚴格模式（[`scope::validate_startup`]）。
///
/// # Errors
///
/// `REGISTRATION_MODE` 不是 `closed` 而 `DATA_API_STRICT_SCOPE` 未開啟時回錯誤。
pub(super) fn check_startup() -> anyhow::Result<()> {
    let registration_mode = std::env::var(scope::REGISTRATION_MODE_ENV).ok();
    scope::validate_startup(registration_mode.as_deref(), scope::strict_scope_enabled())
}
