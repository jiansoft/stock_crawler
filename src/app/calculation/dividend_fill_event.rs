//! 每日重算除權息填息事件衍生表（`dividend_fill_event`）。
//!
//! 條件選股與回測的殖利率、填息率、平均填息天數與連續配息年數條件都讀這張表；05:45 排在 05:35 除權息
//! 公告掃描與 05:37 交易所核對之後，前一晚 21:00 補進的股利與前一個交易日的收盤都已入庫。

use anyhow::Result;

use crate::infra::database::repository::dividend_fill::PgDividendFillRepository;

/// 整批重算 `dividend_fill_event`；失敗時交易回滾，表內保留前一次的結果。
///
/// # Errors
/// 資料庫查詢或寫入失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let rows = PgDividendFillRepository::new()
        .replace_fill_events()
        .await?;
    tracing::info!("重算除權息填息事件完成: rows={rows}");
    Ok(())
}

#[cfg(test)]
mod tests;
