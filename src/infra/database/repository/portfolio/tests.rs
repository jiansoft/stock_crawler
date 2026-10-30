//! 持股倉儲的單元測試。

use rust_decimal_macros::dec;

use super::*;

/// 查詢未售出持股、依序號尋找、更新累積股利、計算與刪除年度已領股利。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn repository_reads_and_updates_holdings() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 repository_reads_and_updates_holdings：無資料庫連接");
        return;
    }
    // 假代號與假會員；已售出的那筆不應出現在未售出清單。
    const SYMBOL: &str = "79979PF";
    const MEMBER: i64 = 979_790;
    let pool = database::get_connection();
    let cleanup = || async {
        let _ = sqlx::query(
            "DELETE FROM dividend_record_detail WHERE stock_ownership_details_serial IN
             (SELECT serial FROM stock_ownership_details WHERE security_code = $1)",
        )
        .bind(SYMBOL)
        .execute(pool)
        .await;
        let _ = sqlx::query("DELETE FROM stock_ownership_details WHERE security_code = $1")
            .bind(SYMBOL)
            .execute(pool)
            .await;
    };
    cleanup().await;

    let insert = |is_sold: bool| async move {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, share_price_average, is_sold)
             VALUES ($1, $2, 1000, 50000, 50, $3) RETURNING serial",
        )
        .bind(MEMBER)
        .bind(SYMBOL)
        .bind(is_sold)
        .fetch_one(pool)
        .await
        .expect("種入持股")
    };
    let serial = insert(false).await;
    insert(true).await;

    let repo = PgPortfolioRepository::new();
    let active = repo
        .fetch_active_holdings(Some(vec![SYMBOL.to_string()]))
        .await
        .expect("查詢未售出持股");
    assert_eq!(active.len(), 1);
    assert_eq!(
        (
            active[0].serial,
            active[0].member_id,
            active[0].share_quantity
        ),
        (serial, MEMBER, 1000)
    );

    let mut holding = repo
        .find_holding_by_serial(serial)
        .await
        .expect("依序號尋找")
        .expect("應找到持股");
    assert_eq!(holding.share_price_average, dec!(50));
    assert!(
        repo.find_holding_by_serial(-1)
            .await
            .expect("查無序號")
            .is_none()
    );

    holding.cumulate_dividends_cash = dec!(3);
    holding.cumulate_dividends_total = dec!(3);
    repo.update_holding_dividends(&holding)
        .await
        .expect("更新累積股利");
    let updated = repo
        .find_holding_by_serial(serial)
        .await
        .expect("讀回持股")
        .expect("應找到持股");
    assert_eq!(
        (
            updated.cumulate_dividends_cash,
            updated.cumulate_dividends_total
        ),
        (dec!(3), dec!(3))
    );

    sqlx::query(
        r#"INSERT INTO dividend_record_detail (stock_ownership_details_serial, "year", cash, stock_money, stock, total)
           VALUES ($1, 1990, 2, 1, 0.1, 3), ($1, 1991, 1, 0, 0, 1)"#,
    )
    .bind(serial)
    .execute(pool)
    .await
    .expect("種入年度已領股利");
    assert_eq!(
        repo.calculate_accumulated_dividends(serial)
            .await
            .expect("計算累積股利"),
        (dec!(3), dec!(0.1), dec!(1), dec!(4))
    );
    repo.delete_received_dividend(serial, 1990)
        .await
        .expect("刪除 1990 年度");
    assert_eq!(
        repo.calculate_accumulated_dividends(serial)
            .await
            .expect("重新計算"),
        (dec!(1), dec!(0), dec!(0), dec!(1))
    );

    cleanup().await;
}
