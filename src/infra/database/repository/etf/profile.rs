//! ETF 清單與基本資料：上市櫃 ETF 清單、Yahoo 基本資料與推斷分類（`etf_profile`）。

use anyhow::{Context, Result};

use crate::{domain::etf::EtfProfileRecord, infra::database};

const LISTED_ETFS_SQL: &str = r#"
SELECT stock_symbol::text, "Name"::text
FROM stocks
WHERE stock_industry_id = 9001 AND NOT "SuspendListing"
ORDER BY stock_symbol
"#;

const SAVE_PROFILE_SQL: &str = r#"
INSERT INTO etf_profile (stock_symbol, full_name, issuer, inception_date, benchmark, category, is_active,
                         region, strategy, themes, dividend_frequency, management_fee, custodian_fee,
                         currency, exchange_name, manager, website, total_assets, total_assets_date,
                         nav_return, source)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21)
ON CONFLICT (stock_symbol) DO UPDATE SET
    full_name = EXCLUDED.full_name,
    issuer = EXCLUDED.issuer,
    inception_date = EXCLUDED.inception_date,
    benchmark = EXCLUDED.benchmark,
    category = EXCLUDED.category,
    is_active = EXCLUDED.is_active,
    region = EXCLUDED.region,
    strategy = EXCLUDED.strategy,
    themes = EXCLUDED.themes,
    dividend_frequency = EXCLUDED.dividend_frequency,
    management_fee = EXCLUDED.management_fee,
    custodian_fee = EXCLUDED.custodian_fee,
    currency = EXCLUDED.currency,
    exchange_name = EXCLUDED.exchange_name,
    manager = EXCLUDED.manager,
    website = EXCLUDED.website,
    total_assets = EXCLUDED.total_assets,
    total_assets_date = EXCLUDED.total_assets_date,
    nav_return = EXCLUDED.nav_return,
    source = EXCLUDED.source,
    updated_at = NOW()
"#;

/// 推斷的分類只補沒有資料的 ETF；已有的推斷列跟著最新的推斷規則更新，Yahoo 寫入的列不動。
const INSERT_MISSING_PROFILE_SQL: &str = r#"
INSERT INTO etf_profile (stock_symbol, full_name, region, strategy, themes, source)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (stock_symbol) DO UPDATE SET
    region = EXCLUDED.region,
    strategy = EXCLUDED.strategy,
    themes = EXCLUDED.themes,
    updated_at = NOW()
WHERE etf_profile.source = 'heuristic'
  AND (etf_profile.region, etf_profile.strategy, etf_profile.themes)
      IS DISTINCT FROM (EXCLUDED.region, EXCLUDED.strategy, EXCLUDED.themes)
"#;

pub(super) async fn listed_etfs() -> Result<Vec<(String, String)>> {
    sqlx::query_as(LISTED_ETFS_SQL)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch listed ETFs")
}

pub(super) async fn save_profiles(profiles: &[EtfProfileRecord]) -> Result<u64> {
    let mut tx = database::get_tx().await?;
    let mut written = 0;
    for p in profiles {
        written += sqlx::query(SAVE_PROFILE_SQL)
            .bind(&p.stock_symbol)
            .bind(&p.full_name)
            .bind(&p.issuer)
            .bind(p.inception_date)
            .bind(&p.benchmark)
            .bind(&p.category)
            .bind(p.is_active)
            .bind(&p.region)
            .bind(&p.strategy)
            .bind(&p.themes)
            .bind(&p.dividend_frequency)
            .bind(&p.management_fee)
            .bind(&p.custodian_fee)
            .bind(&p.currency)
            .bind(&p.exchange_name)
            .bind(&p.manager)
            .bind(&p.website)
            .bind(p.total_assets)
            .bind(p.total_assets_date)
            .bind(&p.nav_return)
            .bind(&p.source)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("Failed to save ETF profile {}", p.stock_symbol))?
            .rows_affected();
    }
    tx.commit().await?;
    Ok(written)
}

pub(super) async fn insert_missing_profiles(profiles: &[EtfProfileRecord]) -> Result<u64> {
    let mut tx = database::get_tx().await?;
    let mut written = 0;
    for p in profiles {
        written += sqlx::query(INSERT_MISSING_PROFILE_SQL)
            .bind(&p.stock_symbol)
            .bind(&p.full_name)
            .bind(&p.region)
            .bind(&p.strategy)
            .bind(&p.themes)
            .bind(&p.source)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("Failed to insert ETF profile {}", p.stock_symbol))?
            .rows_affected();
    }
    tx.commit().await?;
    Ok(written)
}
