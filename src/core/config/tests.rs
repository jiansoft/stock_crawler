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

/// 以固定對照表模擬環境變數。
fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

/// 從環境變數建立設定時的必要值（缺 `SYSTEM_SSL_KEY_FILE`，由各測試補上）。
const REQUIRED: [(&str, &str); 14] = [
    (TELEGRAM_ALLOWED, r#"{"123":"alice"}"#),
    (POSTGRESQL_HOST, "db.local"),
    (POSTGRESQL_USER, "reader"),
    (POSTGRESQL_PASSWORD, "pg-secret"),
    (POSTGRESQL_DB, "stock"),
    (TELEGRAM_TOKEN, "tg-token"),
    (REDIS_ADDR, "redis.local:6379"),
    (REDIS_ACCOUNT, "default"),
    (REDIS_PASSWORD, "redis-secret"),
    (GO_GRPC_TARGET, "https://go.local:9000"),
    (GO_GRPC_TLS_CERT_FILE, "go.pem"),
    (GO_GRPC_TLS_KEY_FILE, "go.key"),
    (GO_GRPC_DOMAIN_NAME, "go.local"),
    (SYSTEM_SSL_CERT_FILE, "fullchain.pem"),
];

/// 必要值全部對應到欄位；選填值未設定時用預設（埠 5432、Redis db 6379、其餘 0 或空字串）。
#[test]
fn from_lookup_maps_required_values_and_defaults() {
    let mut pairs = REQUIRED.to_vec();
    pairs.push((SYSTEM_SSL_KEY_FILE, "privkey.pem"));
    let app = App::from_lookup(&lookup_from(&pairs));

    assert_eq!(app.postgresql.host, "db.local");
    assert_eq!(app.postgresql.port, 5432);
    assert_eq!(app.postgresql.user, "reader");
    assert_eq!(app.postgresql.password, "pg-secret");
    assert_eq!(app.postgresql.db, "stock");
    assert_eq!(app.bot.telegram.token, "tg-token");
    assert_eq!(
        app.bot.telegram.allowed.get(&123).map(String::as_str),
        Some("alice")
    );
    assert_eq!(app.nosql.redis.addr, "redis.local:6379");
    assert_eq!(app.nosql.redis.account, "default");
    assert_eq!(app.nosql.redis.password, "redis-secret");
    assert_eq!(app.nosql.redis.db, 6379);
    assert_eq!(app.nosql.redis.pool_size, 0);
    assert_eq!(app.rpc.go_service.target, "https://go.local:9000");
    assert_eq!(app.rpc.go_service.tls_cert_file, "go.pem");
    assert_eq!(app.rpc.go_service.tls_key_file, "go.key");
    assert_eq!(app.rpc.go_service.domain_name, "go.local");
    assert_eq!(app.system.grpc_use_port, 0);
    assert_eq!(app.system.ssl_cert_file, "fullchain.pem");
    assert_eq!(app.system.ssl_key_file, "privkey.pem");
    assert_eq!(app.fugle.api_key, "");
    assert_eq!(app.logging.file.max_size_mb, 0);
    assert_eq!(app.logging.file.max_age_days, 0);
    assert_eq!(app.logging.seq.server_url, "");
}

/// 選填值有設定時採用；數字解析失敗回預設；Telegram 名單不是 JSON 或空字串時為空。
#[test]
fn from_lookup_parses_optional_values() {
    let mut pairs = REQUIRED.to_vec();
    pairs.retain(|(key, _)| *key != TELEGRAM_ALLOWED);
    pairs.extend([
        (TELEGRAM_ALLOWED, "not json"),
        (SYSTEM_SSL_KEY_FILE, "privkey.pem"),
        (POSTGRESQL_PORT, "abc"),
        (REDIS_DB, "2"),
        (REDIS_POOL_SIZE, "16"),
        (SYSTEM_GRPC_USE_PORT, "9001"),
        (FUGLE_API_KEY, "fugle"),
        (LOG_FILE_MAX_SIZE_MB, " 20 "),
        (LOG_FILE_MAX_AGE_DAYS, "x"),
        (SEQ_SERVER_URL, "http://seq.local"),
        (SEQ_API_KEY, "seq-key"),
    ]);
    let app = App::from_lookup(&lookup_from(&pairs));

    assert!(app.bot.telegram.allowed.is_empty());
    assert_eq!(app.postgresql.port, 5432);
    assert_eq!(app.nosql.redis.db, 2);
    assert_eq!(app.nosql.redis.pool_size, 16);
    assert_eq!(app.system.grpc_use_port, 9001);
    assert_eq!(app.fugle.api_key, "fugle");
    assert_eq!(app.logging.file.max_size_mb, 20);
    assert_eq!(app.logging.file.max_age_days, 0);
    assert_eq!(app.logging.seq.server_url, "http://seq.local");
    assert_eq!(app.logging.seq.api_key, "seq-key");

    pairs.retain(|(key, _)| *key != TELEGRAM_ALLOWED);
    pairs.push((TELEGRAM_ALLOWED, ""));
    assert!(
        App::from_lookup(&lookup_from(&pairs))
            .bot
            .telegram
            .allowed
            .is_empty()
    );
}

/// 缺少必要值時以該名稱 panic，與直接讀環境變數的行為相同。
#[test]
#[should_panic(expected = "SYSTEM_SSL_KEY_FILE")]
fn from_lookup_panics_on_missing_required_value() {
    App::from_lookup(&lookup_from(&REQUIRED));
}

/// 只覆蓋有設定的值；日誌大小解析失敗保留原值，Redis 連線池解析失敗為 0，
/// Telegram 名單不是 JSON 時保留原名單，埠號解析失敗回 5432。
#[test]
fn override_with_replaces_only_present_values() {
    let mut base = App::default();
    base.postgresql.host = "file-host".to_owned();
    base.postgresql.db = "file-db".to_owned();
    base.logging.file.max_size_mb = 30;
    base.logging.file.max_age_days = 14;
    base.nosql.redis.pool_size = 8;
    base.bot.telegram.allowed.insert(1, "file".to_owned());

    let app = base.clone().override_with(&lookup_from(&[
        (FUGLE_API_KEY, "fugle"),
        (SYSTEM_SSL_CERT_FILE, "cert.pem"),
        (SYSTEM_SSL_KEY_FILE, "key.pem"),
        (SEQ_SERVER_URL, "http://seq.local"),
        (SEQ_API_KEY, "seq-key"),
        (LOG_FILE_MAX_SIZE_MB, "abc"),
        (LOG_FILE_MAX_AGE_DAYS, " 3 "),
        (GO_GRPC_TARGET, "https://go.local"),
        (GO_GRPC_TLS_CERT_FILE, "go.pem"),
        (GO_GRPC_TLS_KEY_FILE, "go.key"),
        (GO_GRPC_DOMAIN_NAME, "go.local"),
        (POSTGRESQL_HOST, "env-host"),
        (POSTGRESQL_PORT, "bad"),
        (POSTGRESQL_USER, "env-user"),
        (POSTGRESQL_PASSWORD, "env-password"),
        (TELEGRAM_ALLOWED, "not json"),
        (TELEGRAM_TOKEN, "env-token"),
        (REDIS_ADDR, "redis.local"),
        (REDIS_DB, "x"),
        (REDIS_ACCOUNT, "account"),
        (REDIS_PASSWORD, "redis-secret"),
        (REDIS_POOL_SIZE, "many"),
    ]));

    assert_eq!(app.fugle.api_key, "fugle");
    assert_eq!(app.system.ssl_cert_file, "cert.pem");
    assert_eq!(app.system.ssl_key_file, "key.pem");
    assert_eq!(app.logging.seq.server_url, "http://seq.local");
    assert_eq!(app.logging.seq.api_key, "seq-key");
    assert_eq!(app.logging.file.max_size_mb, 30, "解析失敗保留原值");
    assert_eq!(app.logging.file.max_age_days, 3);
    assert_eq!(app.rpc.go_service.target, "https://go.local");
    assert_eq!(app.rpc.go_service.tls_cert_file, "go.pem");
    assert_eq!(app.rpc.go_service.tls_key_file, "go.key");
    assert_eq!(app.rpc.go_service.domain_name, "go.local");
    assert_eq!(app.postgresql.host, "env-host");
    assert_eq!(app.postgresql.port, 5432);
    assert_eq!(app.postgresql.user, "env-user");
    assert_eq!(app.postgresql.password, "env-password");
    assert_eq!(app.postgresql.db, "file-db", "未設定的值保留設定檔");
    assert_eq!(
        app.bot.telegram.allowed.get(&1).map(String::as_str),
        Some("file")
    );
    assert_eq!(app.bot.telegram.token, "env-token");
    assert_eq!(app.nosql.redis.addr, "redis.local");
    assert_eq!(app.nosql.redis.db, 6379);
    assert_eq!(app.nosql.redis.account, "account");
    assert_eq!(app.nosql.redis.password, "redis-secret");
    assert_eq!(app.nosql.redis.pool_size, 0, "連線池解析失敗回 0");

    let allowed = base.override_with(&lookup_from(&[
        (TELEGRAM_ALLOWED, r#"{"7":"bob"}"#),
        (POSTGRESQL_DB, "env-db"),
    ]));
    assert_eq!(allowed.bot.telegram.allowed.len(), 1);
    assert_eq!(
        allowed.bot.telegram.allowed.get(&7).map(String::as_str),
        Some("bob")
    );
    assert_eq!(allowed.postgresql.db, "env-db");
    assert_eq!(allowed.postgresql.host, "file-host");
}

/// 測試建置以 `TEST_POSTGRESQL_*` 覆蓋資料庫連線；沒設定的欄位不變，埠號解析失敗回 5432。
#[test]
fn override_with_test_replaces_postgresql_connection() {
    let mut base = App::default();
    base.postgresql.host = "prod".to_owned();
    base.postgresql.db = "stock".to_owned();
    let app = base.clone().override_with_test(&lookup_from(&[
        (TEST_POSTGRESQL_HOST, "127.0.0.1"),
        (TEST_POSTGRESQL_PORT, "55433"),
        (TEST_POSTGRESQL_USER, "user"),
        (TEST_POSTGRESQL_PASSWORD, "password"),
        (TEST_POSTGRESQL_DB, "db"),
    ]));
    assert_eq!(app.postgresql.host, "127.0.0.1");
    assert_eq!(app.postgresql.port, 55433);
    assert_eq!(app.postgresql.user, "user");
    assert_eq!(app.postgresql.password, "password");
    assert_eq!(app.postgresql.db, "db");

    let untouched = base.override_with_test(&lookup_from(&[(TEST_POSTGRESQL_PORT, "x")]));
    assert_eq!(untouched.postgresql.host, "prod");
    assert_eq!(untouched.postgresql.port, 5432);
    assert_eq!(untouched.postgresql.db, "stock");
}
