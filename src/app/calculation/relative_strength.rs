//! 收盤後寫入每日相對強度。
//!
//! 回填未來報酬之後，以當天的近 60／120／240 日報酬對上市櫃普通股（不含 ETF、特別股）計分排名，整天替換
//! `daily_relative_strength`（見 [`crate::domain::quote::relative_strength`]）。當天報酬還沒齊
//! （全體計分退回前一個交易日）或不是交易日時不寫入。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::domain::quote::relative_strength::{RelativeStrengthRepository, ScoredUniverseQuery};
use crate::infra::database::repository::quote::PgQuoteRepository;

/// 計算並寫入 `date` 的每日相對強度，回傳寫入列數；當天沒有可計分的資料時回 0。
///
/// # Errors
///
/// 查詢或寫入失敗時回傳錯誤。
pub async fn calculate_relative_strength(date: NaiveDate) -> Result<u64> {
    run(&PgQuoteRepository::new(), date).await
}

/// [`calculate_relative_strength`] 的本體，倉儲可替換以便測試。
pub(crate) async fn run(
    repo: &(impl ScoredUniverseQuery + RelativeStrengthRepository),
    date: NaiveDate,
) -> Result<u64> {
    let Some((data_date, scored)) = repo
        .fetch_scored_universe(date)
        .await
        .context("計算全體相對強度失敗")?
    else {
        return Ok(0);
    };
    if data_date != date {
        return Ok(0);
    }
    repo.replace_relative_strength(date, &scored)
        .await
        .with_context(|| format!("寫入 {date} 的相對強度失敗"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use anyhow::anyhow;
    use async_trait::async_trait;

    use super::*;
    use crate::domain::quote::relative_strength::{ScoredStock, UniverseRow};

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期")
    }

    fn scored(symbol: &str) -> ScoredStock {
        ScoredStock {
            rank: 1,
            rs_score: 99,
            weighted: 12.5,
            row: UniverseRow {
                stock_symbol: symbol.to_owned(),
                name: symbol.to_owned(),
                market_id: 2,
                industry: String::new(),
                return_20d: None,
                return_60d: 0.0,
                return_120d: 0.0,
                return_240d: 0.0,
                closing_price: None,
            },
        }
    }

    /// 回傳固定全體分數、記錄寫入的假倉儲。
    #[derive(Default)]
    struct FakeRepo {
        universe: Option<(NaiveDate, Vec<ScoredStock>)>,
        fail: bool,
        written: Mutex<Vec<(NaiveDate, usize)>>,
    }

    #[async_trait]
    impl ScoredUniverseQuery for FakeRepo {
        async fn fetch_scored_universe(
            &self,
            _until: NaiveDate,
        ) -> Result<Option<(NaiveDate, Vec<ScoredStock>)>> {
            Ok(self.universe.clone())
        }
    }

    #[async_trait]
    impl RelativeStrengthRepository for FakeRepo {
        async fn replace_relative_strength(
            &self,
            date: NaiveDate,
            scored: &[ScoredStock],
        ) -> Result<u64> {
            if self.fail {
                return Err(anyhow!("寫入逾時"));
            }
            self.written
                .lock()
                .expect("測試鎖")
                .push((date, scored.len()));
            Ok(scored.len() as u64)
        }
    }

    /// 當天有分數才整天寫入；退回前一天或沒有資料都不寫；寫入失敗回錯誤。
    #[tokio::test]
    async fn run_writes_only_the_requested_day() {
        let repo = FakeRepo {
            universe: Some((day(14), vec![scored("2330"), scored("2317")])),
            ..Default::default()
        };
        assert_eq!(run(&repo, day(14)).await.expect("成功"), 2);
        assert_eq!(
            run(&repo, day(15)).await.expect("成功"),
            0,
            "退回前一天不寫"
        );
        assert_eq!(repo.written.lock().expect("測試鎖").clone(), [(day(14), 2)]);

        assert_eq!(run(&FakeRepo::default(), day(14)).await.expect("成功"), 0);

        let failing = FakeRepo {
            universe: Some((day(14), vec![scored("2330")])),
            fail: true,
            ..Default::default()
        };
        let error = run(&failing, day(14)).await.expect_err("寫入失敗");
        assert!(format!("{error:#}").contains("2026-10-14"), "{error:#}");
    }
}
