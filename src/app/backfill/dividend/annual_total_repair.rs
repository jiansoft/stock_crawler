//! 修復帶有配息日期的年度合計列。
//!
//! `upsert_annual_total_dividend` 會為「同一發放年度有多次配發」的股票寫入一列
//! `quarter = ''` 的年度合計，日期欄一律填 `'-'`，代表它是明細的加總而不是一次配發。
//!
//! 但股票從年配改成分期配發時，原本那筆 `quarter = ''` 的年度配息明細會與新的合計列
//! 撞上同一組主鍵 `(security_code, year, quarter)`；舊版的 `ON CONFLICT DO UPDATE`
//! 只覆寫金額，於是明細列的除息日與發放日留在合計列上，讓
//! [`crate::app::calculation::dividend_record`] 把合計當成另一次真實配息重複計入。
//!
//! 例：2072 世紀風電 2026 年發放 2025 年配 7.0833 元、2026H1 6 元，合計 13.0833 元，
//! 但殘留日期讓持股明細多算一筆 13.0833 元，領息金額變成 26.17 元。
//!
//! 這個流程掃出所有受影響的 (代號, 發放年度)，重算合計列把日期清回 `'-'`，
//! 再重算對應股票目前持股的已領股利紀錄。整段流程只讀寫資料庫，不會對外部來源發出請求。

use std::collections::HashSet;

use anyhow::Result;

use crate::{
    app::calculation::dividend_record, domain::dividend::repository::DividendRepository,
    infra::database::repository::dividend::PgDividendRepository,
};

/// 年度合計列修復的執行結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnnualTotalRepairSummary {
    /// 掃描到的受影響 (代號, 發放年度) 組數。
    pub stale_count: usize,
    /// 成功重算的年度合計列數。
    pub repaired_count: usize,
    /// 成功重算已領股利紀錄的股票檔數。
    pub record_backfilled_count: usize,
}

/// 掃描並修復所有帶有配息日期的年度合計列。
///
/// 採 best-effort 策略：單一股票失敗只寫 log，不中斷其餘修復，讓一次執行盡可能清乾淨。
pub async fn repair_stale_annual_total_dividends() -> Result<AnnualTotalRepairSummary> {
    let dividend_repo = PgDividendRepository::new();
    let stale = dividend_repo.fetch_stale_annual_total_dividends().await?;

    let mut summary = AnnualTotalRepairSummary {
        stale_count: stale.len(),
        ..Default::default()
    };

    if stale.is_empty() {
        return Ok(summary);
    }

    // 同一檔股票可能有多個年度受影響，已領股利紀錄只需要在全部合計列重算完後補一次。
    let mut record_targets: HashSet<String> = HashSet::new();

    for (security_code, year) in &stale {
        match dividend_repo
            .upsert_annual_total_dividend(security_code, *year)
            .await
        {
            Ok(_) => {
                summary.repaired_count += 1;
                record_targets.insert(security_code.clone());
                tracing::info!(
                    "重算 {} {} 年度合計股利，日期已清回 '-'",
                    security_code,
                    year
                );
            }
            Err(why) => {
                tracing::error!(
                    "重算 {} {} 年度合計股利失敗: {:?}",
                    security_code,
                    year,
                    why
                )
            }
        }
    }

    for security_code in &record_targets {
        match dividend_record::backfill_received_dividend_records_for_stock(security_code).await {
            Ok(_) => summary.record_backfilled_count += 1,
            Err(why) => {
                tracing::error!("重算 {} 的持股已領股利失敗: {:?}", security_code, why)
            }
        }
    }

    Ok(summary)
}
