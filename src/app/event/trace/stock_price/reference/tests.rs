//! 開盤前參考價（除權息、減資恢復買賣）的測試。

use super::*;

/// 沒有除權息與減資事件的日子，兩類參考價都是空集合，整批寫入也不出錯。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn reference_prices_are_empty_on_a_day_without_events() {
    dotenvy::dotenv().ok();
    let date = NaiveDate::from_ymd_opt(1999, 1, 4).unwrap();
    let Ok(resumption) = corporate_action_reference_prices(date).await else {
        println!("跳過 reference_prices_are_empty_on_a_day_without_events：無資料庫連接");
        return;
    };
    assert!(resumption.is_empty());
    assert!(
        ex_rights_reference_prices(date, &resumption)
            .await
            .expect("除權息參考價")
            .is_empty()
    );
    load_reference_prices(date).await;
}

/// 以臨時資料庫跑兩類參考價：減資恢復買賣（前收 ÷ 股數比例）與除權息（同日多筆股利合併；
/// 同日也恢復買賣的以恢復買賣參考價為基準）。假代號 79979R*，固定日期 1999-06-15，結束時刪除。
mod database {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::dividend::entity::ex_rights_reference_price;
    use crate::infra::database;

    /// 減資 5 成恢復買賣，同一天也除息 1 元。
    const RESUMED: &str = "79979RA";
    /// 同一天兩筆股利事件（現金 1＋1 元、股票 1 元）。
    const DOUBLE: &str = "79979RB";
    /// 有減資事件但快取沒有前收，略過。
    const NO_CLOSE: &str = "79979RC";
    const SYMBOLS: [&str; 3] = [RESUMED, DOUBLE, NO_CLOSE];

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(1999, 6, 15).expect("合法日期")
    }

    async fn cleanup() {
        let pool = database::get_connection();
        let symbols: Vec<String> = SYMBOLS.map(String::from).to_vec();
        for sql in [
            "DELETE FROM corporate_action WHERE stock_symbol = ANY($1)",
            "DELETE FROM dividend WHERE security_code = ANY($1)",
            "DELETE FROM last_daily_quotes WHERE stock_symbol = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            sqlx::query(sql)
                .bind(&symbols)
                .execute(pool)
                .await
                .expect("清除測試資料");
        }
    }

    async fn seed() {
        let pool = database::get_connection();
        let ex_date = day().format("%Y-%m-%d").to_string();
        for symbol in SYMBOLS {
            sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol) VALUES ($1, $1, $1)"#,
            )
            .bind(symbol)
            .execute(pool)
            .await
            .expect("插入股票");
        }
        for (symbol, close) in [(RESUMED, dec!(10)), (DOUBLE, dec!(50))] {
            sqlx::query(
                "INSERT INTO last_daily_quotes (date, stock_symbol, closing_price) VALUES ($1, $2, $3)",
            )
            .bind(NaiveDate::from_ymd_opt(1999, 6, 14).expect("合法日期"))
            .bind(symbol)
            .bind(close)
            .execute(pool)
            .await
            .expect("插入前收");
        }
        for symbol in [RESUMED, NO_CLOSE] {
            sqlx::query(
                "INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio, note) \
                 VALUES ($1, $2, 'capital_reduction', 0.5, '測試用')",
            )
            .bind(symbol)
            .bind(day())
            .execute(pool)
            .await
            .expect("插入減資事件");
        }
        for (symbol, quarter, cash, stock, stock_ex) in [
            (RESUMED, "Q1", dec!(1), dec!(0), "-"),
            (DOUBLE, "Q1", dec!(1), dec!(0), "-"),
            (DOUBLE, "Q2", dec!(1), dec!(1), ex_date.as_str()),
        ] {
            sqlx::query(
                r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2)
                   VALUES ($1, 1999, 1998, $2, $3, $4, $3 + $4, $5, $6, '-', '-')"#,
            )
            .bind(symbol)
            .bind(quarter)
            .bind(cash)
            .bind(stock)
            .bind(&ex_date)
            .bind(stock_ex)
            .execute(pool)
            .await
            .expect("插入股利");
        }
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn reference_prices_cover_resumption_and_merged_dividends() {
        dotenvy::dotenv().ok();
        if sqlx::query("SELECT 1")
            .execute(database::get_connection())
            .await
            .is_err()
        {
            println!("跳過 reference_prices_cover_resumption_and_merged_dividends：無資料庫連接");
            return;
        }
        cleanup().await;
        seed().await;
        // 前收從 SHARE 的最後交易日報價讀，重新載入快取讓假代號的前收進去。
        SHARE.load().await;

        let resumption = corporate_action_reference_prices(day()).await;
        let ex_rights = match &resumption {
            Ok(prices) => ex_rights_reference_prices(day(), prices).await,
            Err(_) => Ok(HashMap::new()),
        };
        cleanup().await;
        SHARE.clear_last_trading_day_quotes();

        let resumption = resumption.expect("減資參考價");
        let ex_rights = ex_rights.expect("除權息參考價");
        assert_eq!(resumption.get(RESUMED), Some(&dec!(20)), "前收 10 ÷ 0.5");
        assert!(!resumption.contains_key(NO_CLOSE), "沒有前收的略過");
        assert_eq!(
            ex_rights.get(RESUMED).copied(),
            ex_rights_reference_price(dec!(20), dec!(1), dec!(0)),
            "以恢復買賣參考價 20 為基準除息"
        );
        assert_eq!(
            ex_rights.get(DOUBLE).copied(),
            ex_rights_reference_price(dec!(50), dec!(2), dec!(1)),
            "同日兩筆合併：現金 2 元、股票 1 元"
        );
    }
}
