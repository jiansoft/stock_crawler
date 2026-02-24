use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rust_decimal::Decimal;

use crate::{
    crawler::nstock::{
        self,
        eps::{EpsQuarter, EpsYear},
    },
    database::table::financial_statement::{self, FinancialStatement},
    declare::Quarter,
    logging,
    util::map::Keyable,
};

/// 更新台股年度財報
pub mod annual;
/// 更新台股季度財報
pub mod quarter;

async fn update_roe_and_roa_for_zero_values(quarter: Option<Quarter>) -> Result<()> {
    let fss = financial_statement::fetch_roe_or_roa_equal_to_zero(None, quarter).await?;
    let mut stock_symbols: HashSet<String> = HashSet::new();
    let mut ffs_map: HashMap<String, FinancialStatement> = HashMap::with_capacity(fss.len());

    for fs in fss {
        stock_symbols.insert(fs.security_code.clone());
        ffs_map.insert(fs.key(), fs);
    }

    for stock_symbol in stock_symbols {
        match nstock::eps::visit(&stock_symbol).await {
            Ok(eps) => {
                if quarter.is_none() {
                    update_values_for_years(eps.years, &mut ffs_map).await;
                } else {
                    update_values_for_quarters(eps.quarters, &mut ffs_map).await;
                }
            }
            Err(why) => {
                logging::error_file_async(format!("{:?}", why));
            }
        }
    }

    Ok(())
}

async fn update_values_for_years(
    years: Vec<EpsYear>,
    ffs_map: &mut HashMap<String, FinancialStatement>,
) {
    for year_eps in years {
        let key = year_eps.key();
        if let Some(fs) = ffs_map.get_mut(&key) {
            update_roe_and_roa(fs, year_eps.roe, year_eps.roa).await;
        }
    }
}

async fn update_values_for_quarters(
    quarters: Vec<EpsQuarter>,
    ffs_map: &mut HashMap<String, FinancialStatement>,
) {
    for quarter_eps in quarters {
        let key = quarter_eps.key();
        if let Some(fs) = ffs_map.get_mut(&key) {
            update_roe_and_roa(fs, quarter_eps.roe, quarter_eps.roa).await;
        }
    }
}

async fn update_roe_and_roa(fs: &mut FinancialStatement, roe: Decimal, roa: Decimal) {
    fs.return_on_equity = roe;
    fs.return_on_assets = roa;

    if let Err(why) = fs.update_roe_roa().await {
        logging::error_file_async(format!("{:?}", why));
    }
}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_update_annual_roe_and_roa_for_zero_values() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 update_roe_and_roa_for_zero_values".to_string());

        match update_roe_and_roa_for_zero_values(Some(Quarter::Q3)).await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to roe_and_roa because {:?}", why));
            }
        }

        logging::debug_file_async("結束 update_roe_and_roa_for_zero_values".to_string());
    }
}
