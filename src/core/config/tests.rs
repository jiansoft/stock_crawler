use std::time;

use super::*;

/// 驗證設定可由環境變數與 JSON 載入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_init() {
    dotenvy::dotenv().ok();
    tracing::debug!("SETTINGS.system: {:#?}\r\n", SETTINGS.system);
    tracing::debug!(
        "SETTINGS.postgresql: host={} port={} db={} user={}\r\n",
        SETTINGS.postgresql.host,
        SETTINGS.postgresql.port,
        SETTINGS.postgresql.db,
        SETTINGS.postgresql.user
    );

    tracing::debug!(
        "SETTINGS.nosql.redis: addr={} account={} db={}\r\n",
        SETTINGS.nosql.redis.addr,
        SETTINGS.nosql.redis.account,
        SETTINGS.nosql.redis.db
    );

    tracing::debug!(
        "SETTINGS.rpc.go_service: target={} domain_name={}\r\n",
        SETTINGS.rpc.go_service.target,
        SETTINGS.rpc.go_service.domain_name
    );

    let mut map: HashMap<i64, String> = HashMap::new();
    map.insert(123, "QQ".to_string());
    map.insert(456, "QQ".to_string());
    let json_str = serde_json::to_string(&map).expect("TODO: panic message");

    tracing::debug!("serde_json: {}\r\n", &json_str);
    match serde_json::from_str::<HashMap<i64, String>>(&json_str) {
        Ok(json) => {
            tracing::debug!("json: {:?}\r\n", json);
        }
        Err(why) => {
            tracing::debug!("Failed to serde_json because: {:?} \r\n {}", why, &json_str);
        }
    }
    tokio::time::sleep(time::Duration::from_secs(1)).await;
}

/// 最小可用的設定：只帶必填區塊，選填區塊（`fugle`、`logging`）一律省略。
fn minimal_config() -> serde_json::Value {
    serde_json::json!({
        "bot": { "telegram": {} },
        "postgresql": {},
        "rpc": {
            "go_service": {
                "target": "",
                "tls_cert_file": "",
                "tls_key_file": "",
                "domain_name": ""
            }
        },
        "nosql": { "redis": { "addr": "", "account": "", "password": "", "db": 0 } },
        "system": { "grpc_use_port": 9001, "ssl_cert_file": "", "ssl_key_file": "" }
    })
}

/// 未提供 `logging` 區塊時，各欄位一律為 0／空字串（代表沿用程式預設）。
#[test]
fn logging_section_is_optional_and_defaults_to_zero() {
    let app: App = serde_json::from_value(minimal_config()).expect("最小設定應可解析");

    assert_eq!(app.logging.file.max_size_mb, 0);
    assert_eq!(app.logging.file.max_age_days, 0);
    assert!(app.logging.seq.server_url.is_empty());
    assert!(app.logging.seq.api_key.is_empty());
    assert!(app.fugle.api_key.is_empty());
    assert_eq!(app.system.grpc_use_port, 9001);
}

/// 日誌與 Seq 欄位同時接受 camelCase（實際使用）與 snake_case（alias）。
#[test]
fn logging_fields_accept_both_camel_case_and_snake_case() {
    let mut camel_config = minimal_config();
    camel_config["logging"] = serde_json::json!({
        "file": { "maxSizeMb": 20, "maxAgeDays": 14 },
        "seq": { "serverUrl": "http://localhost:5341", "apiKey": "secret" }
    });
    let mut snake_config = minimal_config();
    snake_config["logging"] = serde_json::json!({
        "file": { "max_size_mb": 20, "max_age_days": 14 },
        "seq": { "server_url": "http://localhost:5341", "api_key": "secret" }
    });

    for config in [camel_config, snake_config] {
        let app: App = serde_json::from_value(config).expect("兩種命名皆應可解析");
        assert_eq!(app.logging.file.max_size_mb, 20);
        assert_eq!(app.logging.file.max_age_days, 14);
        assert_eq!(app.logging.seq.server_url, "http://localhost:5341");
        assert_eq!(app.logging.seq.api_key, "secret");
    }
}

/// 數字欄位寫成字串必須直接解析失敗，而不是被默默轉型。
///
/// 這是刻意的設計：設定檔型別打錯應該在啟動時就爆，而不是等到連不上資料庫。
#[test]
fn numeric_fields_reject_quoted_numbers() {
    let mut config = minimal_config();
    config["postgresql"]["port"] = serde_json::json!("5432");

    let err = serde_json::from_value::<App>(config).expect_err("字串型別的 port 應解析失敗");
    assert!(
        err.to_string().contains("expected i32"),
        "錯誤訊息應指出型別不符：{err}"
    );
}

/// Redis 連線池大小為選填；未提供時為 0（由 infra 層收斂成預設值）。
#[test]
fn redis_pool_size_is_optional() {
    let app: App = serde_json::from_value(minimal_config()).expect("最小設定應可解析");
    assert_eq!(app.nosql.redis.pool_size, 0);

    let mut config = minimal_config();
    config["nosql"]["redis"]["pool_size"] = serde_json::json!(32);
    let app: App = serde_json::from_value(config).expect("含 pool_size 的設定應可解析");
    assert_eq!(app.nosql.redis.pool_size, 32);
}

/// Telegram 允許名單為 `i64 → String` 的對照表，序列化後可原樣還原。
#[test]
fn telegram_allowed_list_round_trips() {
    let mut config = minimal_config();
    config["bot"]["telegram"] = serde_json::json!({ "token": "t", "allowed": { "123": "QQ" } });

    let app: App = serde_json::from_value(config).expect("含允許名單的設定應可解析");
    assert_eq!(app.bot.telegram.allowed.get(&123), Some(&"QQ".to_string()));

    let restored: App = serde_json::from_str(&serde_json::to_string(&app).expect("序列化應成功"))
        .expect("反序列化應成功");
    assert_eq!(restored.bot.telegram.allowed, app.bot.telegram.allowed);
    assert_eq!(restored.bot.telegram.token, "t");
}

/// 設定檔路徑固定為專案根目錄下的 `app.json`。
#[test]
fn config_path_points_at_app_json() {
    assert_eq!(config_path(), PathBuf::from("app.json"));
}
