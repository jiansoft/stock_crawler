//! Data API 查詢預算的連線池路由測試：`statement_timeout` 與 `plan_cache_mode`。

use super::*;

/// 不在 Data API 請求內時沿用全域連線池；作用範圍內改用帶 statement_timeout 的連線池。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn query_budget_routes_to_pools_with_statement_timeout() {
    dotenvy::dotenv().ok();
    let show = || async {
        let timeout: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(get_connection())
            .await
            .unwrap();
        let batch: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(get_batch_connection())
            .await
            .unwrap();
        (timeout, batch)
    };
    assert_eq!(current_query_budget(), None);
    let (global, _) = show().await;
    assert_ne!(global, "6s");
    let (general, batch) = with_query_budget(QueryBudget::General, show()).await;
    assert_eq!((general.as_str(), batch.as_str()), ("6s", "25s"));
    let (heavy, batch) = with_query_budget(QueryBudget::Heavy, show()).await;
    assert_eq!((heavy.as_str(), batch.as_str()), ("25s", "25s"));
    // 超過上限的 statement 被資料庫取消。
    let err = with_query_budget(QueryBudget::General, async {
        sqlx::query("SELECT pg_sleep(7)")
            .execute(get_connection())
            .await
    })
    .await
    .expect_err("6 秒上限應取消查詢");
    assert!(err.to_string().contains("statement timeout"), "{err}");
}

/// Data API 與批次連線池一律 custom plan；一般連線池維持預設的 `auto`。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn api_and_batch_pools_force_custom_plans() {
    dotenvy::dotenv().ok();
    let show = || async {
        let general: String = sqlx::query_scalar("SHOW plan_cache_mode")
            .fetch_one(get_connection())
            .await
            .unwrap();
        let batch: String = sqlx::query_scalar("SHOW plan_cache_mode")
            .fetch_one(get_batch_connection())
            .await
            .unwrap();
        (general, batch)
    };
    let (global, batch) = show().await;
    assert_eq!(
        (global.as_str(), batch.as_str()),
        ("auto", "force_custom_plan")
    );
    for budget in [QueryBudget::General, QueryBudget::Heavy] {
        let (general, batch) = with_query_budget(budget, show()).await;
        assert_eq!(
            (general.as_str(), batch.as_str()),
            ("force_custom_plan", "force_custom_plan"),
            "{budget:?}"
        );
    }
}
