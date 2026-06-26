use anyhow::Result;
use chrono::{Datelike, NaiveDate};

/// 計算便宜、合理、昂貴價的估算
pub async fn calculate_estimated_price(date: NaiveDate) -> Result<()> {
    let years: Vec<i32> = (0..6).map(|i| date.year() - i).collect();
    let years_str = years
        .iter()
        .map(|&year| year.to_string())
        .collect::<Vec<String>>()
        .join(",");

    /*    let stocks = match SHARE.stocks.read() {
        Ok(stocks) => stocks.clone(),
        Err(why) => {
            return Err(anyhow!("Failed to read stocks cache because {:?}", why));
        }
    };*/
    /*let stock_symbols: Vec<String> = stocks.keys().cloned().collect();
     stream::iter(stock_symbols)
    .for_each_concurrent(util::concurrent_limit_32(), |stock_symbol| {
        let years = years_str.clone();
        async move {
            let estimate = Estimate::new(stock_symbol, date);
            if let Err(why) = estimate.upsert(years).await {
                tracing::error!("{:?}", why);
            }
        }
    })
    .await;*/

    use crate::domain::financial::repository::FinancialRepository;
    use crate::infra::database::repository::financial::PgFinancialRepository;

    let financial_repo = PgFinancialRepository::new();
    financial_repo
        .rebuild_price_estimates(date, years_str)
        .await?;

    // 實例化系統設定領域倉儲，用來查詢與更新估值日期設定
    let config_repo = crate::infra::database::repository::config::PgConfigRepository::new();
    use crate::domain::config::entity::SystemConfig;
    use crate::domain::config::repository::ConfigRepository;

    // 取得資料庫中現存的估值日期設定
    let config_opt = config_repo.find_by_key("estimate-date").await?;
    let should_save = match &config_opt {
        Some(cfg) => cfg.should_update_date(date),
        None => true, // 若無設定則必須寫入
    };

    // 只有在需要更新時（新日期大於已存在日期，或尚無設定時）才儲存
    if should_save {
        let new_config = SystemConfig::new(
            "estimate-date".to_string(),
            date.format("%Y-%m-%d").to_string(),
        );
        config_repo.save(&new_config).await?;
    }
    tracing::info!("價格估值日期更新到資料庫完成");

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    use super::*;

    #[tokio::test]
    async fn test_calculate_estimated_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 calculate_estimated_price");
        let current_date = NaiveDate::parse_from_str("2026-03-31", "%Y-%m-%d").unwrap();
        match calculate_estimated_price(current_date).await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to calculate_estimated_price because {:?}", why);
            }
        }
        tracing::debug!("結束 calculate_estimated_price");
    }
}
