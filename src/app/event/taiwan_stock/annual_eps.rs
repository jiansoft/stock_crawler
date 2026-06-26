//! # 台股年度 EPS 補齊事件
//!
//! 此模組負責補齊上一年度尚未寫入的年度 EPS / 年報獲利資料。
//!
//! ## 執行流程
//!
//! 1. 找出資料庫中上一年度尚未有年度資料的股票。
//! 2. 逐一查詢外部來源（富邦、MoneyDJ、MOPS）。
//! 3. 取得資料後轉為 `FinancialStatement` 並寫回資料庫。
//! 4. 將已處理的股票寫入 Redis，避免短時間內重複抓取。

use anyhow::{Result, anyhow};
use chrono::{Datelike, Local, NaiveDate};
use std::collections::HashSet;
use std::time::Duration;

use crate::{app::backfill::acl::FinancialStatementAclMapper, infra::crawler::{
        fbs::annual_profit::Fbs,
        moneydj::annual_profit::MoneyDJ,
        mops::annual_profit::Mops,
        share::{AnnualProfit, AnnualProfitFetcher},
    }};

/// 執行台股年度 EPS 補齊流程。
///
/// 此函式會查詢上一年度缺少年報資料的股票，
/// 並依序向外部來源抓取年度獲利資訊後寫回資料庫。
///
/// # 回傳
/// * `Result<()>` - 成功時表示流程執行完成；
///   失敗時回傳資料庫、Redis 或外部來源相關錯誤。
pub async fn execute() -> Result<()> {
    use crate::domain::financial::repository::FinancialRepository;
    use crate::infra::database::repository::financial::PgFinancialRepository;

    let financial_repo = PgFinancialRepository::new();
    let current_date: NaiveDate = Local::now().date_naive();
    let last_year = current_date.year() - 1;
    let without_annuals = financial_repo
        .fetch_without_annual_statements(last_year)
        .await?;
    let mut stock_symbol: HashSet<String> = HashSet::new();
    for ea in without_annuals {
        stock_symbol.insert(ea.security_code);
    }

    for ss in stock_symbol {
        let cache_key = format!("financial_statement:annual:{}", ss);
        let is_jump = crate::infra::nosql::redis::CLIENT
            .get_bool(&cache_key)
            .await?;
        if is_jump {
            continue;
        }

        match fetch_annual_profit(&ss).await {
            Ok(aps) => {
                for ap in aps {
                    let fs = FinancialStatementAclMapper::from_annual_profit(ap);

                    if let Err(why) = financial_repo.save_annual_eps(&fs).await {
                        tracing::error!("{:?} ", why);
                    }
                }
            }
            Err(why) => {
                tracing::error!("{:?} ", why);
            }
        }

        crate::infra::nosql::redis::CLIENT
            .set(cache_key, true, 60 * 60 * 24 * 7)
            .await?;
        // 主動節流，降低被來源站台限流或封鎖的風險。
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Ok(())
}

/// 依序向多個外部來源取得指定股票的年度獲利資料。
///
/// 目前依序使用：
/// 1. 富邦
/// 2. MoneyDJ
/// 3. MOPS
///
/// 只要任一來源成功且回傳非空資料，即直接回傳結果。
///
/// # 參數
/// * `ss` - 股票代碼。
///
/// # 回傳
/// * `Result<Vec<AnnualProfit>>` - 成功時回傳年度獲利資料集合；
///   若所有來源皆失敗或無資料，則回傳錯誤。
async fn fetch_annual_profit(ss: &str) -> Result<Vec<AnnualProfit>> {
    let sites = vec![Fbs::visit, MoneyDJ::visit, Mops::visit];

    for fetch_func in sites {
        match fetch_func(ss).await {
            Ok(ap) => {
                if ap.is_empty() {
                    continue;
                }

                return Ok(ap);
            }
            Err(why) => {
                tracing::error!("{:?} ", why);
            }
        }
    }

    Err(anyhow!(
        "Failed to fetch annual profit({}) from all sites",
        ss
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("err:{:#?} ", why);
            }
        }

        tracing::debug!("結束 execute");
    }
}
