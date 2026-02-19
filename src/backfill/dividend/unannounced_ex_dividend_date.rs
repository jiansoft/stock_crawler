use anyhow::{anyhow, Result};
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    Retry,
};

use crate::{crawler::yahoo, database::table::dividend, logging};

/// 回補除息/發放日期尚未公布的股利資料。
pub(super) async fn backfill_unannounced_dividend_dates(year: i32) -> Result<()> {
    // 除息日尚未公布
    let dividends =
        dividend::Dividend::fetch_unpublished_dividend_date_or_payable_date_for_specified_year(
            year,
        )
        .await?;

    logging::info_file_async(format!(
        "本次除息日與發放日的採集需收集 {} 家",
        dividends.len()
    ));

    for dividend in dividends {
        if let Err(why) = backfill_unannounced_dividend_dates_from_yahoo(dividend, year).await {
            logging::error_file_async(format!(
                "Failed to backfill_unannounced_dividend_dates_from_yahoo because {:?}",
                why
            ));
        }
    }

    Ok(())
}

/// 從 Yahoo 取得日期欄位，並更新資料庫中的除息/發放日期。
async fn backfill_unannounced_dividend_dates_from_yahoo(
    mut entity: dividend::Dividend,
    year: i32,
) -> Result<()> {
    let strategy = ExponentialBackoff::from_millis(100)
        .map(jitter) // add jitter to delays
        .take(5); // limit to 5 retries
    let retry_future = Retry::spawn(strategy, || yahoo::dividend::visit(&entity.security_code));
    let yahoo = match retry_future.await {
        Ok(yahoo_dividend) => yahoo_dividend,
        Err(why) => {
            return Err(anyhow!("{}", why));
        }
    };

    // 取得今年度的股利數據
    if let Some(yahoo_dividend_details) = yahoo.get_dividend_by_year(year) {
        if let Some(yahoo_dividend_detail) =
            find_changed_dividend_detail(yahoo_dividend_details, &entity)
        {
            entity.ex_dividend_date1 = yahoo_dividend_detail.ex_dividend_date1.to_string();
            entity.ex_dividend_date2 = yahoo_dividend_detail.ex_dividend_date2.to_string();
            entity.payable_date1 = yahoo_dividend_detail.payable_date1.to_string();
            entity.payable_date2 = yahoo_dividend_detail.payable_date2.to_string();

            if let Err(why) = entity.update_dividend_date().await {
                return Err(anyhow!("{}", why));
            }

            logging::info_file_async(format!(
                "dividend update_dividend_date executed successfully. \r\n{:?}",
                entity
            ));
        }
    }

    Ok(())
}

fn find_changed_dividend_detail<'a>(
    details: &'a [yahoo::dividend::YahooDividendDetail],
    entity: &dividend::Dividend,
) -> Option<&'a yahoo::dividend::YahooDividendDetail> {
    details.iter().find(|detail| {
        detail.year_of_dividend == entity.year_of_dividend
            && detail.quarter == entity.quarter
            && (detail.ex_dividend_date1 != entity.ex_dividend_date1
                || detail.ex_dividend_date2 != entity.ex_dividend_date2
                || detail.payable_date1 != entity.payable_date1
                || detail.payable_date2 != entity.payable_date2)
    })
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::cache::SHARE;

    fn sample_detail(
        year: i32,
        year_of_dividend: i32,
        quarter: &str,
        ex1: &str,
        ex2: &str,
        p1: &str,
        p2: &str,
    ) -> yahoo::dividend::YahooDividendDetail {
        yahoo::dividend::YahooDividendDetail {
            year,
            year_of_dividend,
            quarter: quarter.to_string(),
            cash_dividend: dec!(0),
            stock_dividend: dec!(0),
            ex_dividend_date1: ex1.to_string(),
            ex_dividend_date2: ex2.to_string(),
            payable_date1: p1.to_string(),
            payable_date2: p2.to_string(),
        }
    }

    fn sample_entity(year_of_dividend: i32, quarter: &str) -> dividend::Dividend {
        let mut entity = dividend::Dividend::new();
        entity.security_code = "2454".to_string();
        entity.year_of_dividend = year_of_dividend;
        entity.quarter = quarter.to_string();
        entity.ex_dividend_date1 = "2025-07-01".to_string();
        entity.ex_dividend_date2 = "2025-07-02".to_string();
        entity.payable_date1 = "2025-08-01".to_string();
        entity.payable_date2 = "2025-08-02".to_string();
        entity
    }

    #[test]
    fn test_find_changed_dividend_detail_when_dates_changed() {
        let entity = sample_entity(2024, "Q4");
        let details = vec![sample_detail(
            2025,
            2024,
            "Q4",
            "2025-07-10",
            "2025-07-02",
            "2025-08-01",
            "2025-08-02",
        )];

        let found = find_changed_dividend_detail(&details, &entity);
        assert!(found.is_some());
        assert_eq!(found.unwrap().ex_dividend_date1, "2025-07-10");
    }

    #[test]
    fn test_find_changed_dividend_detail_when_dates_unchanged() {
        let entity = sample_entity(2024, "Q4");
        let details = vec![sample_detail(
            2025,
            2024,
            "Q4",
            "2025-07-01",
            "2025-07-02",
            "2025-08-01",
            "2025-08-02",
        )];

        let found = find_changed_dividend_detail(&details, &entity);
        assert!(found.is_none());
    }

    #[test]
    fn test_find_changed_dividend_detail_when_key_not_match() {
        let entity = sample_entity(2024, "Q4");
        let details = vec![sample_detail(
            2025,
            2023,
            "Q3",
            "2025-07-10",
            "2025-07-02",
            "2025-08-01",
            "2025-08-02",
        )];

        let found = find_changed_dividend_detail(&details, &entity);
        assert!(found.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn test_backfill_unannounced_dividend_dates_live() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        let _ = backfill_unannounced_dividend_dates(2025).await;
    }
}
