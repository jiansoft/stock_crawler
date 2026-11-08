//! Data API 的跨層逾時預算：依路由輕重決定請求逾時與資料庫查詢預算。
//!
//! 預算由外而內遞減（Next.js BFF → Go → Rust → PostgreSQL），讓內層先逾時並回明確錯誤，
//! 而不是外層已斷線、內層仍在空轉：
//!
//! | 類型 | BFF | Go | Rust | PostgreSQL |
//! |---|---:|---:|---:|---:|
//! | 一般查詢 | 12s | 10s | 8s | 6s |
//! | 聚合分析／回測 | 35s | 32s | 30s | 25s |
//!
//! 回測上限依 pi 實測：2019 年起的滾動前推、偏移與前緣約 15～18 秒，故不採原規劃的 16 秒。

use std::time::Duration;

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

use super::handlers::error_response;
use crate::{
    core::logging::slow_query,
    infra::database::{self, QueryBudget},
};

/// 一般查詢的 Rust 端請求上限。
pub(crate) const GENERAL_TIMEOUT: Duration = Duration::from_secs(8);

/// 聚合分析與回測的 Rust 端請求上限。
pub(crate) const HEAVY_TIMEOUT: Duration = Duration::from_secs(30);

/// 屬於重型查詢的路徑片段：回測各端點、訊號／法人連買績效、條件選股、穿透曝險、再平衡、
/// ETF 比較、持股績效、持股填息與股利預估。
///
/// 持股填息要讀每檔持股近 5 年的除權息與日線，熱快取約 1 秒，但行程剛重啟時第一次超過 8 秒
/// （2026-10-30 23:17 days=729 被一般上限切斷）。股利預估同樣逐檔讀近 5 年填息，平常 1～2 秒，
/// 2026-11-05 23:2x pi 部署中高負載時超過 10 秒。
const HEAVY_PATHS: [&str; 9] = [
    "/backtest",
    "/performance",
    "/stocks/screen",
    "/portfolio/etf-exposure",
    "/portfolio/rebalance",
    "/portfolio/history",
    "/etfs/compare",
    "/portfolio/dividend-fill",
    "/portfolio/dividend-forecast",
];

/// 依路徑判斷查詢預算。
pub(crate) fn classify(path: &str) -> QueryBudget {
    if HEAVY_PATHS.iter().any(|fragment| path.contains(fragment)) {
        QueryBudget::Heavy
    } else {
        QueryBudget::General
    }
}

/// 該預算的 Rust 端請求上限。
pub(crate) const fn request_timeout(budget: QueryBudget) -> Duration {
    match budget {
        QueryBudget::General => GENERAL_TIMEOUT,
        QueryBudget::Heavy => HEAVY_TIMEOUT,
    }
}

/// 套用請求逾時與資料庫查詢預算；逾時回 504 與標準錯誤 JSON，並記錄慢請求的端點與參數。
pub(crate) async fn enforce(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let budget = classify(&path);
    enforce_with(budget, request_timeout(budget), request, next).await
}

/// [`enforce`] 的本體；逾時長度可注入，讓測試不必真的等 8 秒。
pub(crate) async fn enforce_with(
    budget: QueryBudget,
    timeout: Duration,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    match tokio::time::timeout(
        timeout,
        slow_query::in_api(
            &path,
            database::with_query_budget(budget, next.run(request)),
        ),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => {
            let timeout_secs = timeout.as_secs_f32();
            tracing::warn!(
                "Data API 請求逾時（{}s）: path={path} query={query}",
                timeout_secs
            );
            error_response(
                StatusCode::GATEWAY_TIMEOUT,
                "查詢逾時，請縮小查詢範圍後重試",
            )
        }
    }
}

#[cfg(test)]
mod tests;
