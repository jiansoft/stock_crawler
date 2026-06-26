use anyhow::{Result, anyhow};
use rand::RngExt;
use tokio_retry::{
    Retry,
    strategy::{ExponentialBackoff, jitter},
};

use crate::{
    app::backfill::acl::YahooDividendAclMapper, domain::dividend::repository::DividendRepository,
    infra::crawler::yahoo, infra::database::repository::dividend::PgDividendRepository,
};

/// 回補除息/發放日期尚未公布的股利資料。
///
/// 此函式會查詢資料庫中指定年度除息日或發放日尚未公布的股票，
/// 並透過 Yahoo 財經進行回補。為了避免對 Yahoo 造成負擔而被阻擋，
/// 引入了 Redis 快取（3天效期）與每檔股票 1 秒的延遲節流。
pub(super) async fn backfill_unannounced_dividend_dates(year: i32) -> Result<()> {
    let dividend_repo = PgDividendRepository::new();
    // 從資料庫中取得指定年度內，除息日或發放日尚未公佈的股利資料列表
    let dividends = dividend_repo
        .fetch_unpublished_dividend_date_or_payable_date_for_specified_year(year)
        .await?;

    tracing::info!("本次除息日與發放日的採集需收集 {} 家", dividends.len());

    // 依序遍歷每一檔需要回補的股利資料
    for dividend in dividends {
        let stock_symbol = &dividend.security_code;
        // 建立 Yahoo 股利回補用的 Redis 快取 key。
        // 與 missing_or_multiple.rs 的快取 key 格式保持一致，共享 3 天的快取防護，避免短時間內重複對同一檔股票重複爬取
        let cache_key = format!("yahoo:dividend:{stock_symbol}");

        // 讀取 Redis 快取，確認這檔股票在 3 天內是否已經爬取過
        let is_jump = match crate::infra::nosql::redis::CLIENT
            .get_bool(&cache_key)
            .await
        {
            Ok(val) => val,
            Err(why) => {
                // 若 Redis 讀取失敗，為避免卡住資料回補流程，僅記錄 log 並假設未爬取過 (false)
                tracing::error!(
                    "Failed to get redis cache key {} because {:?}",
                    cache_key,
                    why
                );
                false
            }
        };

        // 如果 3 天內已經抓取過該股票的 Yahoo 股利頁面，則直接跳過，防範高頻請求被封鎖
        if is_jump {
            continue;
        }

        // 先寫入 3 天的快取旗標至 Redis，TTL 設為 3 天 (60秒 * 60分 * 24小時 * 3天 = 259200秒)
        // 這樣做即使單次採集失敗，也能避免下一次排程運行時立刻重複請求
        if let Err(why) = crate::infra::nosql::redis::CLIENT
            .set(&cache_key, true, 60 * 60 * 24 * 3)
            .await
        {
            tracing::error!(
                "Failed to set redis cache key {} because {:?}",
                cache_key,
                why
            );
        }

        // 呼叫 Yahoo 採集器抓取並解析網頁，進而更新資料庫中該股的除息與發放日期
        if let Err(why) = backfill_unannounced_dividend_dates_from_yahoo(dividend, year).await {
            tracing::error!(
                "Failed to backfill_unannounced_dividend_dates_from_yahoo because {:?}",
                why
            );
        }

        // 每檔股票請求完成後，進行隨機 1.5 到 3.0 秒的延遲（Jitter），降低規律請求被 Yahoo WAF 偵測為爬蟲的機率
        let jitter_ms = rand::rng().random_range(1500..=3000);
        tokio::time::sleep(std::time::Duration::from_millis(jitter_ms)).await;
    }

    Ok(())
}

/// 從 Yahoo 取得日期欄位，並更新資料庫中的除息/發放日期。
///
#[allow(deprecated)]
async fn backfill_unannounced_dividend_dates_from_yahoo(
    mut entity: crate::domain::dividend::entity::Dividend,
    year: i32,
) -> Result<()> {
    let dividend_repo = PgDividendRepository::new();
    let strategy = ExponentialBackoff::from_millis(100)
        .map(jitter) // 延遲加入隨機抖動 (Jitter)
        .take(5); // 限制重試次數為 5 次
    // 呼叫 Retry::spawn 開啟重試流程
    let retry_future = Retry::spawn(strategy, || yahoo::dividend::visit(&entity.security_code));
    let yahoo = match retry_future.await {
        Ok(yahoo_dividend) => yahoo_dividend,
        Err(why) => {
            return Err(anyhow!("{}", why));
        }
    };

    // 取得今年度的股利數據
    if let Some(yahoo_dividend_details) = yahoo.get_dividend_by_year(year)
        && let Some(yahoo_dividend_detail) =
            find_changed_dividend_detail(yahoo_dividend_details, &entity)
    {
        let cmd = YahooDividendAclMapper::from_dto(&entity.security_code, yahoo_dividend_detail);
        entity.ex_dividend_date_cash = cmd.ex_dividend_date1;
        entity.ex_dividend_date_stock = cmd.ex_dividend_date2;
        entity.payable_date_cash = cmd.payable_date1;
        entity.payable_date_stock = cmd.payable_date2;

        if let Err(why) = dividend_repo.update_dividend_date(&entity).await {
            return Err(anyhow!("{}", why));
        }

        tracing::info!(
            "dividend update_dividend_date executed successfully. \r\n{:?}",
            entity
        );
    }

    Ok(())
}

fn find_changed_dividend_detail<'a>(
    details: &'a [yahoo::dividend::YahooDividendDetail],
    entity: &crate::domain::dividend::entity::Dividend,
) -> Option<&'a yahoo::dividend::YahooDividendDetail> {
    details.iter().find(|detail| {
        detail.year_of_dividend == entity.year_of_dividend
            && detail.quarter == entity.quarter
            && (detail.ex_dividend_date1 != entity.ex_dividend_date_cash
                || detail.ex_dividend_date2 != entity.ex_dividend_date_stock
                || detail.payable_date1 != entity.payable_date_cash
                || detail.payable_date2 != entity.payable_date_stock)
    })
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::infra::cache::SHARE;

    fn sample_detail(
        year: i32,
        year_of_dividend: i32,
        quarter: &str,
        ex1: &str,
        ex2: &str,
        p1: &str,
        p2: &str,
    ) -> yahoo::dividend::YahooDividendDetail {
        yahoo::dividend::YahooDividendDetail {
            year,
            year_of_dividend,
            quarter: quarter.to_string(),
            cash_dividend: dec!(0),
            stock_dividend: dec!(0),
            ex_dividend_date1: ex1.to_string(),
            ex_dividend_date2: ex2.to_string(),
            payable_date1: p1.to_string(),
            payable_date2: p2.to_string(),
        }
    }

    fn sample_entity(
        year_of_dividend: i32,
        quarter: &str,
    ) -> crate::domain::dividend::entity::Dividend {
        crate::domain::dividend::entity::Dividend {
            serial: 0,
            security_code: "2454".to_string(),
            year: 2025,
            year_of_dividend,
            quarter: quarter.to_string(),
            earnings_cash_dividend: dec!(0),
            capital_reserve_cash_dividend: dec!(0),
            cash_dividend: dec!(0),
            earnings_stock_dividend: dec!(0),
            capital_reserve_stock_dividend: dec!(0),
            stock_dividend: dec!(0),
            sum: dec!(0),
            payout_ratio_cash: dec!(0),
            payout_ratio_stock: dec!(0),
            payout_ratio: dec!(0),
            ex_dividend_date_cash: "2025-07-01".to_string(),
            ex_dividend_date_stock: "2025-07-02".to_string(),
            payable_date_cash: "2025-08-01".to_string(),
            payable_date_stock: "2025-08-02".to_string(),
            created_time: chrono::Local::now(),
            updated_time: chrono::Local::now(),
        }
    }

    #[test]
    fn test_find_changed_dividend_detail_when_dates_changed() {
        let entity = sample_entity(2024, "Q4");
        let details = vec![sample_detail(
            2025,
            2024,
            "Q4",
            "2025-07-10",
            "2025-07-02",
            "2025-08-01",
            "2025-08-02",
        )];

        let found = find_changed_dividend_detail(&details, &entity);
        assert!(found.is_some());
        assert_eq!(found.unwrap().ex_dividend_date1, "2025-07-10");
    }

    #[test]
    fn test_find_changed_dividend_detail_when_dates_unchanged() {
        let entity = sample_entity(2024, "Q4");
        let details = vec![sample_detail(
            2025,
            2024,
            "Q4",
            "2025-07-01",
            "2025-07-02",
            "2025-08-01",
            "2025-08-02",
        )];

        let found = find_changed_dividend_detail(&details, &entity);
        assert!(found.is_none());
    }

    #[test]
    fn test_find_changed_dividend_detail_when_key_not_match() {
        let entity = sample_entity(2024, "Q4");
        let details = vec![sample_detail(
            2025,
            2023,
            "Q3",
            "2025-07-10",
            "2025-07-02",
            "2025-08-01",
            "2025-08-02",
        )];

        let found = find_changed_dividend_detail(&details, &entity);
        assert!(found.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn test_backfill_unannounced_dividend_dates_live() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        let _ = backfill_unannounced_dividend_dates(2025).await;
    }
}
