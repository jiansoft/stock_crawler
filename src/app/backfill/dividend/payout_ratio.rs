use crate::{
    domain::dividend::repository::DividendRepository,
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
/// 「配發的股利 ÷ 同期間每股盈餘」，兩項資料本地都有，因此改為自行計算，不再依賴外部站台。
///
/// 每次只處理 `payout_ratio` 仍為 0 的股利列：已經有值的（例如早年由 Goodinfo 回補的）
/// 不會被覆蓋，財報還沒公布的則維持 0，等下一輪排程重算。
pub async fn execute() -> Result<()> {
    tracing::info!("更新盈餘分配率開始");
    defer! {
       tracing::info!("更新盈餘分配率結束");
    }

    let dividend_repo = PgDividendRepository::new();
    let candidates = dividend_repo.fetch_payout_ratio_candidates().await?;
    let total = candidates.len();

    let ratios: Vec<_> = candidates
        .iter()
        .filter_map(|candidate| candidate.calculate())
        .collect();

    // 首次執行要補算全部歷史股利，一次送上萬筆陣列參數會讓正式機（樹莓派）吃緊，
    // 因此分批寫回；每批各自是一次 UPDATE，中途失敗不影響已完成的批次。
    let mut updated = 0;
    for chunk in ratios.chunks(UPDATE_CHUNK_SIZE) {
        updated += dividend_repo.update_payout_ratios(chunk).await?;
    }

    tracing::info!(
        "更新盈餘分配率完成: candidates={}, calculated={}, updated={}, skipped={}",
        total,
        ratios.len(),
        updated,
        total - ratios.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 payout_ratio::execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to payout_ratio::execute because {:?}", why);
            }
        }

        tracing::debug!("結束 payout_ratio::execute");
    }
}
