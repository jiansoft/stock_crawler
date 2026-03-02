//! # 台股年度 EPS 補齊事件
//!
//! 此模組負責補齊上一年度尚未寫入的年度 EPS / 年報獲利資料。
//!
//! ## 執行流程
//!
//! 1. 找出資料庫中上一年度尚未有年度資料的股票。
//! 2. 逐一查詢外部來源（富邦、元大、MoneyDJ）。
//! 3. 取得資料後轉為 `FinancialStatement` 並寫回資料庫。
//! 4. 將已處理的股票寫入 Redis，避免短時間內重複抓取。

use std::collections::HashSet;

use anyhow::{anyhow, Result};
use chrono::{Datelike, Local, NaiveDate};

use crate::{
    crawler::{
        fbs::annual_profit::Fbs,
        moneydj::annual_profit::MoneyDJ,
        share::{AnnualProfit, AnnualProfitFetcher},
    },
    database::table::{self, financial_statement::FinancialStatement},
    logging, nosql,
};

/// 執行台股年度 EPS 補齊流程。
///
/// 此函式會查詢上一年度缺少年報資料的股票，
/// 並依序向外部來源抓取年度獲利資訊後寫回資料庫。
///
/// # 回傳
/// * `Result<()>` - 成功時表示流程執行完成；
///   失敗時回傳資料庫、Redis 或外部來源相關錯誤。
pub async fn execute() -> Result<()> {
    let current_date: NaiveDate = Local::now().date_naive();
    let last_year = current_date.year() - 1;
    let without_annuals = table::financial_statement::fetch_without_annual(last_year).await?;
    let mut stock_symbol: HashSet<String> = HashSet::new();
    for ea in without_annuals {
        stock_symbol.insert(ea.security_code);
    }

    for ss in stock_symbol {
        let cache_key = format!("financial_statement:annual:{}", ss);
        let is_jump = nosql::redis::CLIENT.get_bool(&cache_key).await?;
        if is_jump {
            continue;
        }

        match fetch_annual_profit(&ss).await {
            Ok(aps) => {
                for ap in aps {
                    let fs = FinancialStatement::from(ap);

                    if let Err(why) = fs.upsert_annual_eps().await {
                        logging::error_file_async(format!("{:?} ", why));
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("{:?} ", why));
            }
        }

        nosql::redis::CLIENT
            .set(cache_key, true, 60 * 60 * 24 * 7)
            .await?;
    }

    Ok(())
}

/// 依序向多個外部來源取得指定股票的年度獲利資料。
///
/// 目前依序使用：
/// 1. 富邦
/// 2. MoneyDJ
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
    let sites = vec![Fbs::visit, MoneyDJ::visit];

    for fetch_func in sites {
        match fetch_func(ss).await {
            Ok(ap) => {
                if ap.is_empty() {
                    continue;
                }

                return Ok(ap);
            }
            Err(why) => {
                logging::error_file_async(format!("{:?} ", why));
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
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("err:{:#?} ", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
