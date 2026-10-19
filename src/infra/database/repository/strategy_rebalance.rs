//! 策略換股紀錄（[`StrategyRebalanceRepository`]）的 PostgreSQL 實作。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::strategy::rebalance::{
    RecordedHolding, StrategyRebalanceRecord, StrategyRebalanceRepository,
};
use crate::infra::database;

/// 寫入一次換股；同策略同一天已有紀錄時不覆蓋。
const SAVE_SQL: &str = r#"
INSERT INTO strategy_rebalance
    (preset_name, date, query, hold_days, etf_weight, cash, market_filtered, symbols, names, weights)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
ON CONFLICT (preset_name, date) DO NOTHING
"#;

/// 指定策略的全部換股紀錄，依換股日由舊到新。
const LIST_SQL: &str = r#"
SELECT date, query, hold_days, etf_weight, cash, market_filtered, symbols, names, weights
FROM strategy_rebalance
WHERE preset_name = $1
ORDER BY date
"#;

/// [`LIST_SQL`] 的一列。
type RecordRow = (
    NaiveDate,
    String,
    i16,
    i16,
    f64,
    bool,
    Vec<String>,
    Vec<Option<String>>,
    Vec<f64>,
);

/// 基於 PostgreSQL `strategy_rebalance` 表的換股紀錄倉儲。
#[derive(Debug, Default, Clone, Copy)]
pub struct PgStrategyRebalanceRepository;

impl PgStrategyRebalanceRepository {
    /// 建立倉儲。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl StrategyRebalanceRepository for PgStrategyRebalanceRepository {
    async fn save(&self, record: &StrategyRebalanceRecord) -> Result<bool> {
        let symbols: Vec<&str> = record
            .holdings
            .iter()
            .map(|holding| holding.symbol.as_str())
            .collect();
        let names: Vec<Option<&str>> = record
            .holdings
            .iter()
            .map(|holding| holding.name.as_deref())
            .collect();
        let weights: Vec<f64> = record
            .holdings
            .iter()
            .map(|holding| holding.weight)
            .collect();
        let result = sqlx::query(SAVE_SQL)
            .bind(&record.preset_name)
            .bind(record.date)
            .bind(&record.query)
            .bind(i16::try_from(record.hold_days).unwrap_or(i16::MAX))
            .bind(i16::from(record.etf_weight))
            .bind(record.cash)
            .bind(record.market_filtered)
            .bind(&symbols)
            .bind(&names)
            .bind(&weights)
            .execute(database::get_connection())
            .await
            .context("寫入策略換股紀錄失敗")?;
        Ok(result.rows_affected() > 0)
    }

    async fn list(&self, preset_name: &str) -> Result<Vec<StrategyRebalanceRecord>> {
        let rows: Vec<RecordRow> = sqlx::query_as(LIST_SQL)
            .bind(preset_name)
            .fetch_all(database::get_connection())
            .await
            .context("讀取策略換股紀錄失敗")?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    date,
                    query,
                    hold_days,
                    etf_weight,
                    cash,
                    market_filtered,
                    symbols,
                    names,
                    weights,
                )| {
                    StrategyRebalanceRecord {
                        preset_name: preset_name.to_owned(),
                        date,
                        query,
                        hold_days: u16::try_from(hold_days).unwrap_or_default(),
                        etf_weight: u8::try_from(etf_weight).unwrap_or_default(),
                        cash,
                        market_filtered,
                        holdings: symbols
                            .into_iter()
                            .zip(names)
                            .zip(weights)
                            .map(|((symbol, name), weight)| RecordedHolding {
                                symbol,
                                name,
                                weight,
                            })
                            .collect(),
                    }
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 寫入後依日期由舊到新讀回；同策略同一天再寫不覆蓋；其他策略的紀錄不混入。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn save_keeps_first_record_and_lists_by_date() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 save_keeps_first_record_and_lists_by_date：無資料庫連接");
            return;
        }
        let name = "79979 換股紀錄測試";
        let cleanup = || async {
            sqlx::query("DELETE FROM strategy_rebalance WHERE preset_name = ANY($1)")
                .bind([name, "79979 其他策略"])
                .execute(database::get_connection())
                .await
                .expect("清除測試資料");
        };
        cleanup().await;
        let day = |d| NaiveDate::from_ymd_opt(1995, 3, d).expect("有效日期");
        let record = |d, symbol: &str| StrategyRebalanceRecord {
            preset_name: name.to_owned(),
            date: day(d),
            query: "from=1995-01-02&limit=2".to_owned(),
            hold_days: 20,
            etf_weight: 50,
            cash: 10.0,
            market_filtered: false,
            holdings: vec![
                RecordedHolding {
                    symbol: symbol.to_owned(),
                    name: Some("甲".to_owned()),
                    weight: 60.0,
                },
                RecordedHolding {
                    symbol: "79979RB".to_owned(),
                    name: None,
                    weight: 30.0,
                },
            ],
        };
        let repository = PgStrategyRebalanceRepository::new();
        assert!(repository.save(&record(20, "79979RA")).await.expect("寫入"));
        assert!(repository.save(&record(1, "79979RC")).await.expect("寫入"));
        assert!(
            !repository
                .save(&record(20, "79979RZ"))
                .await
                .expect("重複寫入")
        );
        assert!(
            repository
                .save(&StrategyRebalanceRecord {
                    preset_name: "79979 其他策略".to_owned(),
                    ..record(5, "79979RA")
                })
                .await
                .expect("其他策略")
        );

        let records = repository.list(name).await.expect("讀取");
        cleanup().await;
        assert_eq!(records, [record(1, "79979RC"), record(20, "79979RA")]);
    }
}
