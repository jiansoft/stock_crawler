use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rust_decimal::Decimal;

use crate::{
    core::declare::Quarter,
    core::util::map::Keyable,
    domain::financial::{
        entity::FinancialStatement as DomainFinancialStatement, repository::FinancialRepository,
    },
    infra::crawler::nstock::{
        self,
        eps::{EpsQuarter, EpsYear},
    },
    infra::database::repository::financial::PgFinancialRepository,
};

/// 更新台股年度財報
pub mod annual;
/// 更新台股季度財報
pub mod quarter;

async fn update_roe_and_roa_for_zero_values(quarter: Option<Quarter>) -> Result<()> {
    let financial_repo = PgFinancialRepository::new();
    let fss = financial_repo
        .fetch_roe_or_roa_equal_to_zero(None, quarter)
        .await?;
    let mut stock_symbols: HashSet<String> = HashSet::new();
    let mut ffs_map: HashMap<String, DomainFinancialStatement> = HashMap::with_capacity(fss.len());

    for fs in fss {
        stock_symbols.insert(fs.security_code.clone());
        let key = format!("{}-{}-{}", fs.security_code, fs.year, fs.quarter);
        ffs_map.insert(key, fs);
    }

    for stock_symbol in stock_symbols {
        match nstock::eps::visit(&stock_symbol).await {
            Ok(eps) => {
                if quarter.is_none() {
                    update_values_for_years(&financial_repo, eps.years, &mut ffs_map).await;
                } else {
                    update_values_for_quarters(&financial_repo, eps.quarters, &mut ffs_map).await;
                }
            }
            Err(why) => {
                tracing::error!("{:?}", why);
            }
        }
    }

    Ok(())
}

async fn update_values_for_years(
    repo: &PgFinancialRepository,
    years: Vec<EpsYear>,
    ffs_map: &mut HashMap<String, DomainFinancialStatement>,
) {
    for year_eps in years {
        let key = year_eps.key();
        if let Some(fs) = ffs_map.get_mut(&key) {
            update_roe_and_roa(repo, fs, year_eps.roe, year_eps.roa).await;
        }
    }
}

async fn update_values_for_quarters(
    repo: &PgFinancialRepository,
    quarters: Vec<EpsQuarter>,
    ffs_map: &mut HashMap<String, DomainFinancialStatement>,
) {
    for quarter_eps in quarters {
        let key = quarter_eps.key();
        if let Some(fs) = ffs_map.get_mut(&key) {
            update_roe_and_roa(repo, fs, quarter_eps.roe, quarter_eps.roa).await;
        }
    }
}

async fn update_roe_and_roa(
    repo: &PgFinancialRepository,
    fs: &mut DomainFinancialStatement,
    roe: Decimal,
    roa: Decimal,
) {
    fs.return_on_equity = roe;
    fs.return_on_assets = roa;

    if let Err(why) = repo.update_statement_roe_roa(fs).await {
        tracing::error!("{:?}", why);
    }
}

#[cfg(test)]
mod tests {
    use crate::{core::logging, infra::cache::SHARE};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_update_annual_roe_and_roa_for_zero_values() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 update_roe_and_roa_for_zero_values");

        match update_roe_and_roa_for_zero_values(Some(Quarter::Q3)).await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to roe_and_roa because {:?}", why);
            }
        }

        tracing::debug!("結束 update_roe_and_roa_for_zero_values");
    }
}
