//! 版本化唯讀股票 Data API 與其線上 OpenAPI 文件。
//!
//! 此模組把 PostgreSQL 的內部 schema 隔離於 HTTP 契約之外；MCP 等內網服務
//! 只需依 `/api-docs/openapi.json` 建立 client，並可從 `/swagger-ui` 互動測試。
//!
//! 檔案分工：`routes` 負責路由註冊與 Bearer 保護範圍、`openapi` 負責 OpenAPI
//! 文件定義、`handlers` 是各 endpoint 的實作、`dto` 是 HTTP 契約型別、
//! `auth` 是市場類端點的 Bearer token 驗證 middleware、`scope` 是持股類端點
//! （`/portfolio/*`）的金鑰與資料範圍 middleware、`lease` 是回測端點的重運算租約 middleware、
//! `analysis_gate` 是外部請求與內部排程共用的重運算並行號誌。

mod analysis_gate;
mod auth;
mod budget;
mod dto;
mod handlers;
mod lease;
mod openapi;
mod routes;
mod scope;

pub use handlers::BacktestStrategyGateway;
pub(crate) use handlers::validate_strategy_query;
pub(super) use routes::router;

/// 啟動檢查：開放註冊時需要嚴格持股範圍，以及實測後設定的採集與重運算上限。
///
/// # Errors
///
/// 開放註冊缺少嚴格範圍或有效上限時回錯誤。
pub(super) fn check_startup() -> anyhow::Result<()> {
    let registration_mode = std::env::var(scope::REGISTRATION_MODE_ENV).ok();
    scope::validate_startup(registration_mode.as_deref(), scope::strict_scope_enabled())?;
    let mode = registration_mode.as_deref().map(str::trim).unwrap_or("");
    if !mode.is_empty() && !mode.eq_ignore_ascii_case("closed") {
        analysis_gate::validate_open_limits(|name| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.trim().parse().ok())
        })?;
    }
    Ok(())
}
