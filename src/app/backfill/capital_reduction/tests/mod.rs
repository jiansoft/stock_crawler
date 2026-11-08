//! 減資與分割事件回補的測試。

use super::*;

mod mock_sources;
use crate::domain::performance::CorporateActionType;
use rust_decimal_macros::dec;

fn sample(symbol: &str) -> CorporateAction {
    CorporateAction {
        stock_symbol: symbol.to_string(),
        effective_date: NaiveDate::from_ymd_opt(1990, 1, 1).expect("測試日期應合法"),
        action_type: CorporateActionType::CapitalReduction,
        share_ratio: dec!(0.7),
        note: "減資彌補虧損".to_string(),
    }
}

/// 起始日必須正好是端點允許的最早日。早一天就會被 TWSE 拒絕，
/// 全量回補會一筆都寫不進去（2026-09-18 就是這樣踩到的）。
#[test]
fn full_backfill_start_matches_endpoint_lower_bound() {
    assert_eq!(
        full_backfill_start(),
        NaiveDate::from_ymd_opt(2011, 1, 1).expect("測試日期應合法")
    );

    let (year, month, day) = twse::capital_reduction::EARLIEST_QUERYABLE_DATE;
    assert_eq!(
        full_backfill_start(),
        NaiveDate::from_ymd_opt(year, month, day).expect("端點下限應為合法日期")
    );
}

/// 安全閥：超過上限時整批放棄，且不得碰資料庫。
#[tokio::test]
async fn save_all_rejects_oversized_batch() {
    let actions: Vec<CorporateAction> = (0..=MAX_ACTIONS_PER_RUN)
        .map(|index| sample(&format!("{index:05}")))
        .collect();

    let result = save_all(actions, "測試").await;

    assert!(result.is_err(), "超過安全閥應回傳錯誤");
    let message = format!("{:#}", result.expect_err("已斷言為錯誤"));
    assert!(
        message.contains("安全閥"),
        "錯誤訊息應說明是安全閥擋下的：{message}"
    );
}

/// 空批次不應被安全閥擋下，也不該產生任何寫入。
#[tokio::test]
async fn save_all_accepts_empty_batch() {
    let saved = save_all(Vec::new(), "測試").await.expect("空批次應成功");

    assert_eq!(saved, 0);
}

/// 逐筆寫入並回傳成功筆數；單筆失敗（代號超過欄位長度）只記錄，不影響其他筆。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn save_all_writes_each_action_and_skips_failures() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 save_all_writes_each_action_and_skips_failures：無資料庫連接");
        return;
    }
    let cleanup = || async {
        sqlx::query("DELETE FROM corporate_action WHERE stock_symbol LIKE '79979C_'")
            .execute(pool)
            .await
            .expect("清理測試資料");
    };
    cleanup().await;

    let mut split = sample("79979CB");
    split.action_type = CorporateActionType::Split;
    split.share_ratio = dec!(4);
    split.note = "1:4 分割".to_string();
    let actions = vec![sample("79979CA"), sample(&"9".repeat(40)), split];
    let saved = save_all(actions, "測試").await.expect("部分失敗仍成功");

    let rows: Vec<(String, String, rust_decimal::Decimal)> = sqlx::query_as(
        "SELECT stock_symbol, action_type, share_ratio FROM corporate_action
         WHERE stock_symbol LIKE '79979C_' ORDER BY stock_symbol",
    )
    .fetch_all(pool)
    .await
    .expect("讀回公司行動");
    cleanup().await;

    assert_eq!(saved, 2);
    assert_eq!(
        rows,
        [
            (
                "79979CA".to_owned(),
                "capital_reduction".to_owned(),
                dec!(0.7)
            ),
            ("79979CB".to_owned(), "split".to_owned(), dec!(4)),
        ]
    );
}
