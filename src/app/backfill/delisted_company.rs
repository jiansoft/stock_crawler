use std::collections::HashSet;

use crate::{
    app::backfill::acl::DelistedCompanyAclMapper, core::declare::StockExchangeMarket,
    core::util::datetime::Weekend, domain::registry::repository::StockRepository,
    infra::crawler::twse, infra::database::repository::stock::PgStockRepository,
};
use anyhow::{Context, Result};
use chrono::Local;
use scopeguard::defer;

/// 單一市場 ISIN 名冊的合理最小筆數（安全閥）。
///
/// 名冊筆數低於此值代表來源頁面可能改版、被擋或回傳殘缺資料，
/// 此時直接放棄差集比對——寧可這一輪不標記，也不要把整個市場誤判為下市。
const MIN_ROSTER_SIZE_PER_MARKET: usize = 50;

/// 排程單輪允許自動標記下市的最大檔數（安全閥）。
///
/// 正常情況下每天新增的下市檔數是個位數；差集一旦超過此值，
/// 幾乎可以肯定是名冊來源異常（例如漏了某個分類），而不是真的大量下市。
const MAX_DELIST_PER_RUN: usize = 100;

/// 更新資料庫中終止上市櫃的證券。
///
/// 兩階段：
/// 1. TWSE `suspendListingCsvAndHtml`——官方「終止上市」名單（只涵蓋上市公司）。
/// 2. ISIN 名冊差集——補足第一階段的盲區：終止**上櫃**（TPEX 沒有對應的
///    OpenAPI 名單）與 ETF 下架/清算都不在 TWSE 名單內，但它們會從
///    ISIN 現行名冊中消失，因此「資料庫未下市、卻不在任何市場名冊上」
///    的證券即可推定已下市。
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }
    tracing::info!("更新下市的股票開始");
    defer! {
       tracing::info!("更新下市的股票結束");
    }
    let delisted = twse::suspend_listing::visit().await?;
    let repo = PgStockRepository::new();

    for company in delisted {
        // 透過防腐層轉譯為內部命令，內含格式與民國年分過濾邏輯
        if let Some(cmd) = DelistedCompanyAclMapper::from_suspend_listing(&company)
            && let Some(stock) = repo.find_by_symbol(&cmd.symbol).await?
        {
            if stock.suspend_listing() {
                continue;
            }

            let mut another = stock.clone();
            another.update_suspension(true);

            if let Err(why) = repo.save(&another).await {
                tracing::error!("Failed to update_suspend_listing because {:?}", why);
            }
        }
    }

    // 第二階段獨立執行、失敗不影響第一階段的成果——
    // TWSE 名單已寫入的下市標記不因 ISIN 來源異常而回滾。
    if let Err(why) = mark_stocks_missing_from_isin_roster(MAX_DELIST_PER_RUN).await {
        tracing::error!(
            "mark_stocks_missing_from_isin_roster failed: error={:#}",
            why
        );
    }

    Ok(())
}

/// 以 ISIN 現行名冊的「差集」找出已下市但未被標記的證券。
///
/// # 原理
/// `isin.twse.com.tw` 的名冊只列「目前掛牌中」的證券（涵蓋上市、上櫃、
/// 興櫃、公開發行，含 ETF）。資料庫裡 `SuspendListing = false` 卻不在
/// 名冊上的證券，代表它已經終止上市櫃或清算——這正是 TWSE 終止上市
/// API 蓋不到的部分（終止上櫃、ETF 下架）。
///
/// # 安全設計（為什麼不直接相信差集）
/// 差集比對的風險是「名冊不完整 → 大量誤標下市」，因此有三道防線：
/// 1. 任一市場的名冊抓取失敗 → 整個階段放棄（回傳錯誤）。
/// 2. 任一市場的名冊筆數異常少（< [`MIN_ROSTER_SIZE_PER_MARKET`]）→ 放棄。
/// 3. 差集檔數超過 `max_delist` → 放棄並回報，讓人工確認是真的大量下市
///    還是來源異常。首次執行若歷史積欠超過上限，請用手動入口
///    （`app::manual_backfill` 的對應測試）以較高上限清理。
///
/// # 參數
/// - `max_delist`：本輪允許自動標記的最大檔數；排程用 [`MAX_DELIST_PER_RUN`]，
///   手動清理歷史資料時可放寬。
///
/// # 回傳
/// 成功時回傳本輪實際標記下市的檔數。
pub async fn mark_stocks_missing_from_isin_roster(max_delist: usize) -> Result<usize> {
    // 逐市場收集現行名冊；用 HashSet 讓後面的差集查找是 O(1)。
    let mut roster: HashSet<String> = HashSet::with_capacity(4096);
    // 記下名冊實際涵蓋的市場編號——資料庫中其他市場（若未來新增）的
    // 證券不在名冊範圍內，不可以拿來比對，否則必然被誤判。
    let mut covered_market_ids: HashSet<i32> = HashSet::new();

    for market in StockExchangeMarket::iterator() {
        // 注意：一定要用「全類別」名冊（visit_all_listed_symbols）而不是
        // 股票主檔用的 visit()——後者只保留股票/特別股/TDR 類別，
        // ETF 不在其中，直接拿來差集會把所有 ETF 誤判為已下市。
        let symbols =
            twse::international_securities_identification_number::visit_all_listed_symbols(market)
                .await
                .with_context(|| format!("fetch isin roster failed: market={:?}", market))?;

        // 防線 2：名冊異常小代表來源可能改版、被擋或適逢週末（來源回空），
        // 放棄本輪比對。
        if symbols.len() < MIN_ROSTER_SIZE_PER_MARKET {
            anyhow::bail!(
                "isin roster suspiciously small: market={:?}, size={}（放棄差集比對，避免大量誤標下市）",
                market,
                symbols.len()
            );
        }

        covered_market_ids.insert(market.serial());
        roster.extend(symbols);
    }

    let repo = PgStockRepository::new();
    // 只比對「未下市且市場在名冊涵蓋範圍內」的證券。
    let missing: Vec<_> = repo
        .fetch_all_active()
        .await
        .context("fetch_all_active failed")?
        .into_iter()
        .filter(|stock| covered_market_ids.contains(&stock.market_id()))
        .filter(|stock| !roster.contains(&stock.symbol().0))
        .collect();

    if missing.is_empty() {
        return Ok(0);
    }

    // 防線 3：差集數量異常大時放棄自動標記，留給人工確認。
    if missing.len() > max_delist {
        anyhow::bail!(
            "ISIN 名冊差集達 {} 檔（上限 {}），疑似名冊來源異常，放棄本輪自動標記。差集樣本: {:?}",
            missing.len(),
            max_delist,
            missing
                .iter()
                .take(10)
                .map(|stock| stock.symbol().0.clone())
                .collect::<Vec<_>>()
        );
    }

    let mut marked = 0usize;
    for stock in missing {
        let mut another = stock.clone();
        another.update_suspension(true);

        match repo.save(&another).await {
            Ok(_) => {
                marked += 1;
                // 標記下市屬於低頻但重要的狀態變更，用 info 留下可追查的紀錄。
                tracing::info!(
                    "已標記下市（ISIN 名冊差集）: {} {}（market_id={}）",
                    stock.symbol().0,
                    stock.name(),
                    stock.market_id()
                );
            }
            Err(why) => {
                tracing::error!(
                    "Failed to mark stock as delisted: {} because {:#}",
                    stock.symbol().0,
                    why
                );
            }
        }
    }

    Ok(marked)
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {
                tracing::debug!("execute executed successfully.");
            }
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
    }

    /// 手動入口：以 ISIN 名冊差集清理歷史積欠的未標記下市證券。
    ///
    /// 首次執行時，多年未被標記的終止上櫃與 ETF 清算標的可能超過排程的
    /// 單輪上限（[`MAX_DELIST_PER_RUN`]），此測試以放寬的上限一次清完。
    /// 執行前建議先跑一次並檢查 log 中列出的標的是否合理。
    #[tokio::test]
    #[ignore = "live test：連線真實外部網站並寫入資料庫，需要時手動執行"]
    async fn test_mark_stocks_missing_from_isin_roster_manual_cleanup() {
        dotenvy::dotenv().ok();
        SHARE.load().await;

        match mark_stocks_missing_from_isin_roster(1000).await {
            Ok(marked) => println!("已標記 {marked} 檔下市證券"),
            Err(why) => println!("差集清理失敗: {why:#}"),
        }
    }
}
