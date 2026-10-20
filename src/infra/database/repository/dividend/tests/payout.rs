//! 盈餘分配率（含涵蓋期間）寫入與讀回的整合測試。

use super::*;

/// 清掉測試代號寫進 financial_statement 的財報。
async fn cleanup_earnings() -> Result<()> {
    sqlx::query("DELETE FROM financial_statement WHERE security_code = $1")
        .bind(MIXED_YEAR_SYMBOL)
        .execute(database::get_connection())
        .await?;
    Ok(())
}

/// 盈餘分配率從讀取、計算到寫回走一遍，確認欄位型別與 SQL 都對得上
/// （`financial_statement.year` 是 bigint，曾因直接解碼成 i32 讓排程失敗）。
///
/// 仿 4735 豪展的季配息：同一發放年度有 2019Q3、2019Q4、2020Q1 三筆，
/// 年度合計的分母是三者涵蓋期間 EPS 的合計（2019Q1~Q3 1.7 + 2019Q4 0.74 + 2020Q1 1.19）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn payout_ratios_round_trip_with_covered_period() {
    use crate::domain::dividend::payout::calculate_payout_ratios;

    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    cleanup().await.expect("測試前清理失敗");
    cleanup_earnings().await.expect("測試前清理財報失敗");

    for (year, quarter, eps) in [
        (2019_i64, "Q1", dec!(0.29)),
        (2019, "Q2", dec!(0.51)),
        (2019, "Q3", dec!(0.9)),
        (2019, "Q4", dec!(0.74)),
        (2020, "Q1", dec!(1.19)),
    ] {
        sqlx::query(
            "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share) VALUES ($1, $2, $3, $4)",
        )
        .bind(MIXED_YEAR_SYMBOL)
        .bind(year)
        .bind(quarter)
        .bind(eps)
        .execute(database::get_connection())
        .await
        .expect("測試財報寫入失敗");
    }
    for dividend in [
        build_dividend(
            MIXED_YEAR_SYMBOL,
            2019,
            "Q3",
            dec!(0.7),
            "2019-12-02",
            "2020-01-10",
        ),
        build_dividend(
            MIXED_YEAR_SYMBOL,
            2019,
            "Q4",
            dec!(0.5),
            "2020-04-10",
            "2020-04-29",
        ),
        build_dividend(
            MIXED_YEAR_SYMBOL,
            2020,
            "Q1",
            dec!(1),
            "2020-06-15",
            "2020-07-08",
        ),
    ] {
        repo.save(&dividend).await.expect("測試資料寫入失敗");
    }
    repo.upsert_annual_total_dividend(MIXED_YEAR_SYMBOL, TEST_PAYOUT_YEAR)
        .await
        .expect("年度合計重算失敗");

    let (dividends, earnings) = repo
        .fetch_payout_ratio_inputs()
        .await
        .expect("讀取分配率計算資料失敗");
    let ours: Vec<_> = dividends
        .into_iter()
        .filter(|row| row.security_code == MIXED_YEAR_SYMBOL)
        .collect();
    let ratios = calculate_payout_ratios(&ours, &earnings);
    assert_eq!(
        ratios.len(),
        4,
        "三筆配息加一列年度合計都要算出來：{ratios:?}"
    );
    repo.update_payout_ratios(&ratios)
        .await
        .expect("寫回分配率失敗");

    let (payout_eps, payout_period, payout_ratio): (Option<Decimal>, Option<String>, Decimal) =
        sqlx::query_as(
            "SELECT payout_eps, payout_period, payout_ratio FROM dividend WHERE security_code = $1 AND year = $2 AND quarter = ''",
        )
        .bind(MIXED_YEAR_SYMBOL)
        .bind(TEST_PAYOUT_YEAR)
        .fetch_one(database::get_connection())
        .await
        .expect("年度合計列讀取失敗");
    assert_eq!(payout_eps, Some(dec!(3.63)));
    assert_eq!(payout_period.as_deref(), Some("2019Q1~2020Q1"));
    assert_eq!(payout_ratio, dec!(60.6061));

    // 再算一次不該有任何列需要更新。
    let (dividends, earnings) = repo
        .fetch_payout_ratio_inputs()
        .await
        .expect("第二次讀取失敗");
    let ours: Vec<_> = dividends
        .into_iter()
        .filter(|row| row.security_code == MIXED_YEAR_SYMBOL)
        .collect();
    assert!(calculate_payout_ratios(&ours, &earnings).is_empty());

    cleanup().await.expect("測試後清理失敗");
    cleanup_earnings().await.expect("測試後清理財報失敗");
}
