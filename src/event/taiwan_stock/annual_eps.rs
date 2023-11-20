use std::collections::HashSet;

use anyhow::{anyhow, Result};
use chrono::{Datelike, Local, NaiveDate};

use crate::{
    crawler::{
        fbs::annual_profit::Fbs,
        moneydj::annual_profit::MoneyDJ,
        share::{AnnualProfit, AnnualProfitFetcher},
        yuanta::annual_profit::YuanTa,
    },
    database::table::{self, financial_statement::FinancialStatement},
    logging, nosql,
};

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

async fn fetch_annual_profit(ss: &str) -> Result<Vec<AnnualProfit>> {
    let sites = vec![Fbs::visit, YuanTa::visit, MoneyDJ::visit];

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
