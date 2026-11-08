//! 股利資料表查詢的測試。

use chrono::TimeZone;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_no_dividends_for_year() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_no_dividends_for_year");
    let r = Dividend::fetch_no_dividends_for_year(2023).await;
    if let Ok(result) = r {
        for e in result {
            tracing::debug!("{:?} ", e);
        }
    } else if let Err(err) = r {
        tracing::debug!("{:#?} ", err);
    }
    tracing::debug!("結束 fetch_no_dividends_for_year");
}

#[tokio::test]

async fn test_fetch_unpublished_dividend_date_or_payable_date_for_specified_year() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_unpublished_dividend_date_or_payable_date_for_specified_year");
    let r =
        Dividend::fetch_unpublished_dividend_date_or_payable_date_for_specified_year(2023).await;
    if let Ok(result) = r {
        tracing::debug!("{:#?} ", result);
    } else if let Err(err) = r {
        tracing::debug!("{:#?} ", err);
    }
    tracing::debug!(
        "{}",
        "結束 fetch_unpublished_dividend_date_or_payable_date_for_specified_year".to_string(),
    );
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_multiple_dividends_for_year() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_multiple_dividends_for_year");
    let r = Dividend::fetch_multiple_dividends_for_year(2023).await;
    if let Ok(result) = r {
        for e in result {
            tracing::debug!("{:?} ", e);
        }
    } else if let Err(err) = r {
        tracing::debug!("{:#?} ", err);
    }
    tracing::debug!("結束 fetch_multiple_dividends_for_year");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_dividends_summary_by_date() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_dividends_summary_by_date");

    let datetime = Local.with_ymd_and_hms(2022, 3, 9, 0, 0, 0).unwrap();

    let r = Dividend::fetch_dividends_summary_by_date("2330", 2022, datetime).await;
    if let Ok(result) = r {
        for e in result {
            tracing::debug!("{:?} ", e);
        }
    } else if let Err(err) = r {
        tracing::debug!("{:#?} ", err);
    }
    tracing::debug!("結束 fetch_dividends_summary_by_date");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_yearly_dividends_sum_by_date() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_yearly_dividends_sum_by_date");
    let mut e = Dividend::new();
    e.security_code = "2887".to_string();
    e.year = 2022;
    let datetime = Local.with_ymd_and_hms(2022, 3, 9, 0, 0, 0).unwrap();

    let r = e.fetch_yearly_dividends_sum_by_date(datetime).await;
    if let Ok(result) = r {
        tracing::debug!("{:?} {:?}", e, result);
    } else if let Err(err) = r {
        tracing::debug!("{:#?} ", err);
    }
    tracing::debug!("結束 fetch_yearly_dividends_sum_by_date");
}

/// 以臨時資料庫測發放年度清單與依買進日的年度合計；假代號 79979DQ，結束時刪除。
mod fake_dividends {
    use rust_decimal_macros::dec;

    use super::*;

    const SYMBOL: &str = "79979DQ";

    async fn cleanup() {
        sqlx::query("DELETE FROM dividend WHERE security_code = $1")
            .bind(SYMBOL)
            .execute(database::get_connection())
            .await
            .expect("清除測試股利");
    }

    /// 1998 年一筆年配；1999 年兩期（03-10 現金 1 元、06-10 現金 2＋股票 0.5 元）加一筆年度合計列。
    async fn seed() {
        for (year, quarter, cash, stock, ex_date) in [
            (1998, "", dec!(0.8), dec!(0), "1998-07-01"),
            (1999, "Q1", dec!(1), dec!(0), "1999-03-10"),
            (1999, "Q2", dec!(2), dec!(0.5), "1999-06-10"),
            (1999, "", dec!(3), dec!(0.5), "1999-06-10"),
        ] {
            sqlx::query(
                r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2)
                   VALUES ($1, $2, $2 - 1, $3, $4, $5, $4 + $5, $6, '-', '-', '-')"#,
            )
            .bind(SYMBOL)
            .bind(year)
            .bind(quarter)
            .bind(cash)
            .bind(stock)
            .bind(ex_date)
            .execute(database::get_connection())
            .await
            .expect("插入股利");
        }
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn years_and_yearly_sum_follow_purchase_date() {
        dotenvy::dotenv().ok();
        if sqlx::query("SELECT 1")
            .execute(database::get_connection())
            .await
            .is_err()
        {
            println!("跳過 years_and_yearly_sum_follow_purchase_date：無資料庫連接");
            return;
        }
        cleanup().await;
        seed().await;
        let dividend = Dividend {
            security_code: SYMBOL.to_string(),
            year: 1999,
            ..Default::default()
        };
        let purchased = |month: u32, day: u32| {
            Local
                .with_ymd_and_hms(1999, month, day, 0, 0, 0)
                .single()
                .expect("合法時間")
        };

        let years = Dividend::fetch_years_by_security_code(SYMBOL).await;
        let whole_year = dividend
            .fetch_yearly_dividends_sum_by_date(purchased(1, 1))
            .await;
        let after_first = dividend
            .fetch_yearly_dividends_sum_by_date(purchased(5, 1))
            .await;
        let after_all = dividend
            .fetch_yearly_dividends_sum_by_date(purchased(7, 1))
            .await;
        cleanup().await;

        assert_eq!(years.expect("發放年度"), vec![1998, 1999]);
        // 年度合計列與分期並存時只算分期。
        assert_eq!(whole_year.expect("全年"), (dec!(3), dec!(0.5), dec!(3.5)));
        assert_eq!(
            after_first.expect("第二期"),
            (dec!(2), dec!(0.5), dec!(2.5))
        );
        assert_eq!(
            after_all.expect("買在兩期之後"),
            (dec!(0), dec!(0), dec!(0))
        );
    }
}
