//! 持股類端點的金鑰與資料範圍（會員系統計畫 5.0.1）：嚴格模式下市場金鑰回 401、缺 household_id
//! 回 400、成員不屬於家庭回 403；相容模式下舊呼叫方式照常可用。市場類端點不收個人金鑰。

use super::support::pool_or_skip;
use super::*;

const PERSONAL_KEY: &str = "stocks-integration-personal-key";
const MARKET_KEY: &str = "stocks-integration-market-key";

/// 測試期間設定兩把金鑰與相容開關，結束（含 panic）時還原成原本的值。
struct EnvGuard(Vec<(&'static str, Option<String>)>);

impl EnvGuard {
    fn set(values: &[(&'static str, Option<&str>)]) -> Self {
        let saved = values
            .iter()
            .map(|(name, _)| (*name, std::env::var(name).ok()))
            .collect();
        for (name, value) in values {
            // --test-threads=1 下沒有其他測試同時讀寫環境變數
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
        Self(saved)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
    }
}

async fn status(path: &str, key: &str) -> StatusCode {
    router()
        .oneshot(
            Request::get(path)
                .header("Authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request")
        .status()
}

/// 嚴格模式：只收個人金鑰、household_id 必填、指定的會員必須屬於該家庭；市場端點不收個人金鑰。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn strict_scope_rejects_market_key_missing_household_and_foreign_member() {
    let Some(pool) =
        pool_or_skip("strict_scope_rejects_market_key_missing_household_and_foreign_member").await
    else {
        return;
    };
    let _env = EnvGuard::set(&[
        ("DATA_API_PERSONAL_KEY", Some(PERSONAL_KEY)),
        ("DATA_API_KEY", Some(MARKET_KEY)),
        ("DATA_API_STRICT_SCOPE", Some("true")),
    ]);
    sqlx::query(r#"DELETE FROM "Member" WHERE "Account" = '__scope_member__'"#)
        .execute(pool)
        .await
        .expect("清除殘留測試會員");
    let member: i64 = sqlx::query_scalar(
        r#"INSERT INTO "Member" ("Account", "Name", household_id)
           VALUES ('__scope_member__', '__scope_member__', 1) RETURNING "Id""#,
    )
    .fetch_one(pool)
    .await
    .expect("建立測試會員");

    let summary = "/api/v1/portfolio/summary";
    assert_eq!(
        status(&format!("{summary}?household_id=1"), MARKET_KEY).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(status(summary, PERSONAL_KEY).await, StatusCode::BAD_REQUEST);
    assert_eq!(
        status(&format!("{summary}?member_id={member}"), PERSONAL_KEY).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status(&format!("{summary}?household_id=1"), PERSONAL_KEY).await,
        StatusCode::OK
    );
    assert_eq!(
        status(
            &format!("{summary}?household_id=1&member_id={member}"),
            PERSONAL_KEY
        )
        .await,
        StatusCode::OK
    );
    // 會員不屬於這個家庭（不存在的家庭、不存在的會員）
    assert_eq!(
        status(
            &format!("{summary}?household_id=979905&member_id={member}"),
            PERSONAL_KEY
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(
            &format!("{summary}?household_id=1&member_id=979888"),
            PERSONAL_KEY
        )
        .await,
        StatusCode::FORBIDDEN
    );
    // 寫入端點的範圍放在 JSON body：一樣檢查
    let put = router()
        .oneshot(
            Request::put("/api/v1/portfolio/targets")
                .header("Authorization", format!("Bearer {PERSONAL_KEY}"))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "household_id": 979905,
                        "member_id": member,
                        "dimension": "stock",
                        "targets": [],
                    })
                    .to_string(),
                ))
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(put.status(), StatusCode::FORBIDDEN);
    // 市場類端點只收市場金鑰
    assert_eq!(
        status("/api/v1/market/regime", PERSONAL_KEY).await,
        StatusCode::UNAUTHORIZED
    );

    sqlx::query(r#"DELETE FROM "Member" WHERE "Id" = $1"#)
        .bind(member)
        .execute(pool)
        .await
        .expect("清除測試會員");
}

/// 相容模式（預設）：以市場金鑰、不帶 household_id 的舊呼叫方式照常可用；帶了 household_id 仍照新規則驗證。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn compat_scope_keeps_legacy_callers_working() {
    if pool_or_skip("compat_scope_keeps_legacy_callers_working")
        .await
        .is_none()
    {
        return;
    }
    let _env = EnvGuard::set(&[
        ("DATA_API_PERSONAL_KEY", Some(PERSONAL_KEY)),
        ("DATA_API_KEY", Some(MARKET_KEY)),
        ("DATA_API_STRICT_SCOPE", None),
    ]);
    let summary = "/api/v1/portfolio/summary";
    assert_eq!(status(summary, MARKET_KEY).await, StatusCode::OK);
    assert_eq!(
        status(&format!("{summary}?member_id=979888"), MARKET_KEY).await,
        StatusCode::OK
    );
    assert_eq!(status(summary, PERSONAL_KEY).await, StatusCode::OK);
    assert_eq!(
        status(
            &format!("{summary}?household_id=1&member_id=979888"),
            MARKET_KEY
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(status(summary, "wrong-key").await, StatusCode::UNAUTHORIZED);
}
