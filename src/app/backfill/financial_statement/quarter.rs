//! 台股季度財報補欄位流程。
//!
//! 此模組負責找出指定季度中 `ROE`、`ROA` 或每股淨值為零的財報資料，
//! 再到 Yahoo 財經補抓對應欄位並回寫資料庫。
//!
//! 目標季度必須與季 EPS 主流程一致，因此同樣使用
//! [`crate::util::datetime::backfill_report_quarter_targets_for_listed_and_otc`]
//! 依上市/上櫃公司季報法定申報截止日與較保守的預抓視窗決定處理清單。

use std::future::Future;

use anyhow::Result;
use chrono::Local;
use futures::StreamExt;

use crate::{
    app::backfill::acl::FinancialStatementAclMapper,
    app::backfill::financial_statement::update_roe_and_roa_for_zero_values,
    app::calculation,
    core::util::datetime::ReportQuarter,
    domain::financial::{entity::FinancialStatement, repository::FinancialRepository},
    infra::crawler::yahoo,
    infra::database::repository::financial::PgFinancialRepository,
};

/// 補齊最新應已公告季度財報中缺漏的 ROE、ROA 與每股淨值欄位。
///
/// 本流程會先找出正式季度與預抓季度中數值為零的財報，再逐筆到 Yahoo 財經
/// 抓取對應資料，回寫 `financial_statement` 後，重新同步 `stocks` 表上的
/// 最新一季/近四季數據，並觸發估值重算。
pub async fn execute() -> Result<()> {
    let mut success_count = 0usize;
    // 記錄實際處理過的季度，流程結束後才派發通知事件。
    // 這個排程（04:00）跑在季 EPS 主流程（03:00）之後，補完 ROE/ROA 的資料才算完整，
    // 在這裡發事件，通知看到的才不是半套數字。
    let mut processed_quarters: Vec<ReportQuarter> = Vec::new();

    for target_report in
        crate::core::util::datetime::backfill_report_quarter_targets_for_listed_and_otc(Local::now())
    {
        success_count += process_target_report(target_report).await?;
        processed_quarters.push(target_report);

        if let Err(why) = update_roe_and_roa_for_zero_values(Some(target_report.quarter)).await {
            tracing::error!("{:#?}", why);
        }
    }

    if success_count > 0 {
        // 實例化證券主檔倉儲，更新 EPS 與 ROE 數據
        let stock_repo = crate::infra::database::repository::stock::PgStockRepository::new();
        use crate::domain::registry::repository::StockRepository;
        stock_repo.update_eps_and_roe().await?;
        // 實例化系統設定領域倉儲，用來查詢價格估值日期設定
        let config_repo = crate::infra::database::repository::config::PgConfigRepository::new();
        use crate::domain::config::repository::ConfigRepository;

        // 取得價格估值日期 "estimate-date" 設定值
        let config_opt = config_repo.find_by_key("estimate-date").await?;
        let date = match config_opt {
            Some(cfg) => cfg.parse_val_as_date()?,
            None => {
                return Err(anyhow::anyhow!(
                    "無法在系統設定中找到 'estimate-date' 的設定值"
                ));
            }
        };
        // 計算便宜、合理、昂貴價的估算
        calculation::estimated_price::calculate_estimated_price(date).await?;
        tracing::info!("季度財報更新重新計算便宜、合理、昂貴價的估算結束");
    }

    dispatch_quarterly_financials_updated(&processed_quarters).await;

    Ok(())
}

/// 派發季報更新事件，供背景送出持股財報通知。
///
/// 事件只帶年度與季度，要通知哪些股票由 handler 依持股自行查詢。
/// 通知屬於旁路副作用，即使沒有任何持股命中也不影響本流程。
async fn dispatch_quarterly_financials_updated(processed_quarters: &[ReportQuarter]) {
    if processed_quarters.is_empty() {
        return;
    }

    let events: Vec<crate::domain::events::DomainEvent> = processed_quarters
        .iter()
        .map(
            |report| crate::domain::events::DomainEvent::QuarterlyFinancialsUpdated {
                year: report.year,
                quarter: report.quarter.to_string(),
                occurred_at: Local::now(),
            },
        )
        .collect();

    crate::app::event::get_global_dispatcher()
        .dispatch_async(events)
        .await;
}

/// 補齊單一目標季度內缺漏的 Yahoo 財務欄位。
///
/// 使用 `futures::stream` 併發呼叫 Yahoo 財經爬取 Profile 資料，
/// 爬取完成後再一次批量 Upsert 入庫，大幅減少資料庫連線延遲，並降低 Yahoo 連線之 sequential 阻塞。
async fn process_target_report(target_report: ReportQuarter) -> Result<usize> {
    let financial_repo = PgFinancialRepository::new();

    // 找出該季度中 ROE 或 ROA 為 0 的財報清單
    let fss = financial_repo
        .fetch_roe_or_roa_equal_to_zero(Some(target_report.year), Some(target_report.quarter))
        .await?;
    fill_from_yahoo_profile(
        &financial_repo,
        target_report,
        fss,
        |stock_symbol| async move { yahoo::profile::visit(&stock_symbol).await },
    )
    .await
}

/// 逐筆向 Yahoo 取 profile，批次寫回後記下成功快取。
///
/// `fetch` 是 Yahoo profile 的查詢函式，測試時換成假資料。
async fn fill_from_yahoo_profile<F, Fut>(
    financial_repo: &PgFinancialRepository,
    target_report: ReportQuarter,
    fss: Vec<FinancialStatement>,
    fetch: F,
) -> Result<usize>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<yahoo::profile::Profile>>,
{
    let quarter = target_report.quarter.to_string();
    let candidates = fss.len();
    let fetch = &fetch;

    // 建立併發爬取的 async stream，並限制併發數量上限為 5
    let mut stream = futures::stream::iter(fss)
        .map(|fs| {
            let target_report: ReportQuarter = target_report;
            let quarter = quarter.clone();
            async move {
                let cache_key = format!("FinancialStatement:{}-{}-{}", fs.security_code, fs.year, fs.quarter);
                let profile_skip_cache_key = yahoo::profile::no_valid_data_cache_key(&fs.security_code);

                // 檢查 Redis 狀態，決定是否跳過（例如先前已處理過，或是已知無有效資料的股票）
                let is_jump = match crate::infra::nosql::redis::CLIENT.get_bool(&cache_key).await {
                    Ok(val) => val,
                    Err(why) => {
                        tracing::error!("Redis error getting {}: {:?}", cache_key, why);
                        false
                    }
                };
                let is_profile_skip = match crate::infra::nosql::redis::CLIENT.get_bool(&profile_skip_cache_key).await {
                    Ok(val) => val,
                    Err(why) => {
                        tracing::error!("Redis error getting {}: {:?}", profile_skip_cache_key, why);
                        false
                    }
                };

                if is_jump || is_profile_skip {
                    return None;
                }

                // 呼叫 Yahoo API 抓取 Profile 資訊
                let profile = match fetch(fs.security_code.clone()).await {
                    Ok(profile) => profile,
                    Err(why) => {
                        // 查無有效資料或個股頁 404（已下市）都是預期狀況：寫入 Redis 排除快取避免重複查詢
                        if let Some(ttl) = yahoo::profile::skip_cache_ttl_seconds(&why) {
                            if let Err(cache_err) = crate::infra::nosql::redis::CLIENT
                                .set(&profile_skip_cache_key, true, ttl)
                                .await
                            {
                                tracing::error!("Failed to cache yahoo::profile skip for {} because {:?}",
                                    fs.security_code, cache_err);
                            }
                            tracing::warn!("Skip yahoo::profile::visit for {} for {} seconds because {}",
                                fs.security_code, ttl, why);
                        } else {
                            tracing::error!("Failed to yahoo::profile::visit for {} because {}",
                                fs.security_code, why);
                        }
                        return None;
                    }
                };

                // 比對資料年份與季度是否與目標相符，若不符則記錄警告但依原邏輯仍繼續處理
                if target_report.year != profile.year || quarter != profile.quarter {
                    tracing::warn!("the year or quarter retrieved from Yahoo is inconsistent with the current one. current year:{} ,quarter:{} {:#?}",
                        target_report.year, quarter, profile);
                }

                // 建立新的領域財報結構體
                let new_fs = FinancialStatementAclMapper::from_yahoo_profile(profile);
                Some((new_fs, cache_key))
            }
        })
        .buffer_unordered(5);

    let mut scraped_statements = Vec::new();
    let mut keys_to_cache = Vec::new();

    // 收集所有成功爬取的結果
    while let Some(res) = stream.next().await {
        if let Some((new_fs, cache_key)) = res {
            scraped_statements.push(new_fs);
            keys_to_cache.push(cache_key);
        }
    }

    let success_count = scraped_statements.len();

    // 如果有成功爬取的財報資料，執行批量 Upsert 寫入資料庫，並更新對應的 Redis 快取
    if !scraped_statements.is_empty() {
        // 批量寫入資料庫
        financial_repo
            .batch_save_financial_statements(&scraped_statements)
            .await?;
        tracing::debug!(
            "financial_statement batch_upsert executed successfully for {} records.",
            scraped_statements.len()
        );

        // 逐一更新 Redis 成功狀態快取
        for cache_key in keys_to_cache {
            if let Err(why) = crate::infra::nosql::redis::CLIENT
                .set(&cache_key, true, 60 * 60 * 24 * 7)
                .await
            {
                tracing::error!(
                    "Failed to set redis cache key {} because {:?}",
                    cache_key,
                    why
                );
            }
        }
    }

    tracing::info!(
        "Yahoo profile 補季報欄位結束: report={}-{quarter} candidates={candidates} updated={success_count}",
        target_report.year
    );

    Ok(success_count)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use anyhow::anyhow;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use crate::{
        core::declare::Quarter,
        infra::{
            cache::SHARE, crawler::yahoo::dividend::YahooPageNotFoundError, database,
            nosql::redis::CLIENT,
        },
    };

    use super::*;

    /// 假代號：C 補得到、D 的個股頁 404、E 是一般錯誤。
    const SYMBOL_C: &str = "79979RC";
    const SYMBOL_D: &str = "79979RD";
    const SYMBOL_E: &str = "79979RE";
    const SYMBOLS: [&str; 3] = [SYMBOL_C, SYMBOL_D, SYMBOL_E];
    /// 固定的歷史季度，不使用今天的日期當資料日期。
    const REPORT: ReportQuarter = ReportQuarter {
        year: 2020,
        quarter: Quarter::Q1,
    };

    fn success_cache_key(symbol: &str) -> String {
        format!("FinancialStatement:{symbol}-2020-Q1")
    }

    async fn cleanup() {
        let pool = database::get_connection();
        for symbol in SYMBOLS {
            for sql in [
                "DELETE FROM financial_statement WHERE security_code = $1",
                "DELETE FROM stocks WHERE stock_symbol = $1",
            ] {
                let _ = sqlx::query(sql).bind(symbol).execute(pool).await;
            }
            let _ = CLIENT.delete(&success_cache_key(symbol)).await;
            let _ = CLIENT
                .delete(&yahoo::profile::no_valid_data_cache_key(symbol))
                .await;
        }
    }

    /// 2020-Q1 ROE、ROA 都是 0 的財報。
    fn zero_statement(symbol: &str) -> FinancialStatement {
        FinancialStatement {
            security_code: symbol.to_owned(),
            year: 2020,
            quarter: "Q1".to_owned(),
            net_asset_value_per_share: dec!(30),
            earnings_per_share: dec!(2),
            ..FinancialStatement::default()
        }
    }

    async fn pending_rows(repo: &PgFinancialRepository) -> Vec<FinancialStatement> {
        repo.fetch_roe_or_roa_equal_to_zero(Some(REPORT.year), Some(REPORT.quarter))
            .await
            .expect("fetch_roe_or_roa_equal_to_zero")
            .into_iter()
            .filter(|fs| SYMBOLS.contains(&fs.security_code.as_str()))
            .collect()
    }

    /// 補到的寫回並記成功快取；404 記排除快取；一般錯誤不記，下一輪只再查它。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn fill_from_yahoo_profile_saves_found_profiles_and_caches_outcomes() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
            println!(
                "跳過 fill_from_yahoo_profile_saves_found_profiles_and_caches_outcomes：無資料庫或 Redis 連接"
            );
            return;
        }
        cleanup().await;
        for symbol in SYMBOLS {
            sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, 'profile 補值測試', $1, 2, false)"#,
            )
            .bind(symbol)
            .execute(database::get_connection())
            .await
            .expect("種入股票主檔");
        }
        let repo = PgFinancialRepository::new();
        repo.batch_save_financial_statements(&SYMBOLS.map(zero_statement))
            .await
            .expect("種入假代號財報");

        let calls = Mutex::new(Vec::new());
        let fetch = |symbol: String| {
            calls.lock().expect("calls lock").push(symbol.clone());
            async move {
                match symbol.as_str() {
                    SYMBOL_C => Ok(yahoo::profile::Profile {
                        quarter: "Q1".to_owned(),
                        stock_symbol: symbol.clone(),
                        net_asset_value_per_share: dec!(31),
                        earnings_per_share: dec!(2),
                        return_on_equity: dec!(6),
                        return_on_assets: dec!(4),
                        year: 2020,
                        ..yahoo::profile::Profile::default()
                    }),
                    SYMBOL_D => Err(anyhow::Error::new(YahooPageNotFoundError {
                        stock_symbol: symbol.clone(),
                        url: "https://example.invalid".to_owned(),
                    })),
                    _ => Err(anyhow!("Yahoo 測試失敗")),
                }
            }
        };

        let saved = fill_from_yahoo_profile(&repo, REPORT, pending_rows(&repo).await, &fetch)
            .await
            .expect("fill_from_yahoo_profile");
        assert_eq!(saved, 1);
        let (roe, roa): (Decimal, Decimal) = sqlx::query_as(
            "SELECT return_on_equity, return_on_assets FROM financial_statement WHERE security_code = $1 AND year = 2020 AND quarter = 'Q1'",
        )
        .bind(SYMBOL_C)
        .fetch_one(database::get_connection())
        .await
        .expect("讀回假代號財報");
        assert_eq!((roe, roa), (dec!(6), dec!(4)));
        assert!(
            CLIENT
                .get_bool(&success_cache_key(SYMBOL_C))
                .await
                .unwrap_or(false)
        );
        assert!(
            CLIENT
                .get_bool(&yahoo::profile::no_valid_data_cache_key(SYMBOL_D))
                .await
                .unwrap_or(false)
        );
        assert!(
            !CLIENT
                .get_bool(&yahoo::profile::no_valid_data_cache_key(SYMBOL_E))
                .await
                .unwrap_or(false)
        );

        // 第二輪 C 已補好不在待補清單、D 被排除快取擋住，只剩 E 再查一次。
        calls.lock().expect("calls lock").clear();
        let saved = fill_from_yahoo_profile(&repo, REPORT, pending_rows(&repo).await, &fetch)
            .await
            .expect("fill_from_yahoo_profile");
        assert_eq!(saved, 0);
        assert_eq!(
            *calls.lock().expect("calls lock"),
            vec![SYMBOL_E.to_owned()]
        );

        cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
    }
}
