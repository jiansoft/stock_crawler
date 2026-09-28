use crate::{
    domain::dividend::{payout::calculate_payout_ratios, repository::DividendRepository},
    infra::database::repository::dividend::PgDividendRepository,
};
use anyhow::Result;
use scopeguard::defer;

/// 單次寫回的股利列數上限。
const UPDATE_CHUNK_SIZE: usize = 500;

/// <summary>
/// 依財報每股盈餘計算股利的盈餘分配率並寫回。
/// </summary>
///
/// 盈餘分配率原本向 Goodinfo 取得，但該站已對機器請求全面掛上 Cloudflare 瀏覽器驗證，
/// 連自動化瀏覽器都過不了，排程只會每天固定失敗一次。這個比率的定義就是
/// 「配發的股利 ÷ 這筆股利涵蓋期間的每股盈餘」，兩項資料本地都有，因此改為自行計算
/// （涵蓋期間的規則見 [`crate::domain::dividend::payout`]）。
///
/// 每次都重算全部股利列、只寫回結果有變的列：年度合計會隨新的一季配息而改變，
/// 涵蓋期間也會因為補進較早的配息而縮短，只算一次（例如只處理分配率為 0 的列）
/// 就會留下過期的值。涵蓋期間的財報還沒公布的列維持原值，等下一輪排程重算。
pub async fn execute() -> Result<()> {
    tracing::info!("更新盈餘分配率開始");
    defer! {
       tracing::info!("更新盈餘分配率結束");
    }

    let dividend_repo = PgDividendRepository::new();
    let (dividends, earnings) = dividend_repo.fetch_payout_ratio_inputs().await?;
    let total = dividends.len();

    let ratios = calculate_payout_ratios(&dividends, &earnings);

    // 首次執行要補算全部歷史股利，一次送上萬筆陣列參數會讓正式機（樹莓派）吃緊，
    // 因此分批寫回；每批各自是一次 UPDATE，中途失敗不影響已完成的批次。
    let mut updated = 0;
    for chunk in ratios.chunks(UPDATE_CHUNK_SIZE) {
        updated += dividend_repo.update_payout_ratios(chunk).await?;
    }

    tracing::info!(
        "更新盈餘分配率完成: dividends={}, changed={}, updated={}",
        total,
        ratios.len(),
        updated
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    use super::*;

    /// 手動重算全部盈餘分配率（會寫入 `.env` 指向的資料庫）。
    /// 失敗必須讓測試失敗：只記 debug 日誌的話，執行者看到 ok 會以為已經寫入。
    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;

        execute().await.expect("payout_ratio::execute 失敗");
    }
}
