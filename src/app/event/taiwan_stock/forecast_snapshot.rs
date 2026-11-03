//! 每月 1 日 08:15 股利預估快照：把各會員未來 12 個月的逐筆預估（與 `/portfolio/dividend-forecast`
//! 同一套規則）寫進 `dividend_forecast_snapshot`，事後由 `/portfolio/dividend-forecast/accuracy`
//! 與實際入帳比對預估誤差。
//!
//! 同一天重跑時整批替換；沒有持股的會員不記錄。

use anyhow::Result;
use chrono::{Local, NaiveDate};

use crate::{
    domain::portfolio::dividend_forecast, infra::database::repository::portfolio::forecast,
};

/// 快照涵蓋的月數，與 `/portfolio/dividend-forecast` 的預設相同。
const MONTHS: u32 = 12;

/// 排程入口：記錄今天的股利預估快照。
///
/// # Errors
///
/// 查詢持股、配息或寫入快照失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let count = record(Local::now().date_naive()).await?;
    tracing::info!("股利預估快照寫入 {count} 筆");
    Ok(())
}

/// 以 `today` 為快照日，依各會員目前持股預估未來 [`MONTHS`] 個月並整批寫入，回傳寫入筆數。
async fn record(today: NaiveDate) -> Result<u64> {
    let mut items = Vec::new();
    for member_id in forecast::fetch_member_ids().await? {
        let inputs = forecast::load_inputs(member_id, today, MONTHS).await?;
        let result = dividend_forecast::forecast(
            &inputs.holdings,
            &inputs.events,
            &inputs.entitlements,
            today,
            MONTHS,
        );
        items.extend(result.items.into_iter().map(|item| (member_id, item)));
    }
    forecast::replace_snapshot(today, &items).await
}

#[cfg(test)]
mod tests;
