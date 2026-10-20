use std::{collections::HashMap, fs, path::PathBuf, str::FromStr};

use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

mod env_override;

const CONFIG_PATH: &str = "app.json";

/// 應用程式總組態結構體
///
/// 此結構體整合了所有子模組的設定項目，包含資料庫、通訊協定與系統行為。
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct App {
    /// Fugle 行情 API 設定
    #[serde(default)]
    pub fugle: Fugle,
    /// 機器人 (如 Telegram) 設定
    pub bot: Bot,
    /// PostgreSQL 資料庫連線設定
    pub postgresql: PostgreSQL,
    /// RPC 通訊服務設定
    pub rpc: Rpc,
    /// NoSQL (如 Redis) 儲存設定
    pub nosql: NoSQL,
    /// 系統全域行為設定
    pub system: System,
    /// 日誌輸出與外部收集器設定
    #[serde(default)]
    pub logging: Logging,
}

const SYSTEM_GRPC_USE_PORT: &str = "SYSTEM_GRPC_USE_PORT";
const SYSTEM_SSL_CERT_FILE: &str = "SYSTEM_SSL_CERT_FILE";
const SYSTEM_SSL_KEY_FILE: &str = "SYSTEM_SSL_KEY_FILE";

/// 系統全域設定項
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct System {
    /// gRPC 服務所使用的埠號
    pub grpc_use_port: i32,
    /// SSL 憑證檔案路徑
    pub ssl_cert_file: String,
    /// SSL 私鑰檔案路徑
    pub ssl_key_file: String,
}

const SEQ_SERVER_URL: &str = "SEQ_SERVER_URL";
const SEQ_API_KEY: &str = "SEQ_API_KEY";
const LOG_FILE_MAX_SIZE_MB: &str = "LOG_FILE_MAX_SIZE_MB";
const LOG_FILE_MAX_AGE_DAYS: &str = "LOG_FILE_MAX_AGE_DAYS";

/// 日誌相關設定項。
///
/// 涵蓋本地輪轉檔案（`file`）與外部日誌收集器（`seq`）。
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Logging {
    /// 本地輪轉日誌檔設定。
    #[serde(default)]
    pub file: FileLogging,
    /// Seq 日誌收集器設定。
    #[serde(default)]
    pub seq: SeqLogging,
}

/// 本地輪轉日誌檔設定。
///
/// 兩個欄位皆為 `0`（或未設定）時，沿用 `core::logging::rotate` 的預設值
/// （單檔 10 MB、保留 7 天）。實際生效值會在套用時收斂至安全範圍。
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct FileLogging {
    /// 單一日誌檔大小上限（MB）；超過即輪轉出新檔（檔名帶時間戳）。
    #[serde(default, rename = "maxSizeMb", alias = "max_size_mb")]
    pub max_size_mb: u64,
    /// 日誌檔保留天數；超過此天數的舊檔會在跨日切檔時被刪除。
    #[serde(default, rename = "maxAgeDays", alias = "max_age_days")]
    pub max_age_days: i64,
}

/// Seq 日誌收集器連線設定。
///
/// `server_url` 為 Seq 伺服器根網址；`api_key` 為送入事件時使用的 API Key。
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct SeqLogging {
    /// Seq 伺服器根網址，例如 `http://localhost:5341`。
    #[serde(default, rename = "serverUrl", alias = "server_url")]
    pub server_url: String,
    /// Seq API Key；空字串代表不附帶 API Key。
    #[serde(default, rename = "apiKey", alias = "api_key")]
    pub api_key: String,
}

/// RPC 服務入口設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Rpc {
    /// Go 語言實作的後端服務設定
    pub go_service: Grpc,
}

const GO_GRPC_TARGET: &str = "GO_GRPC_TARGET";
const GO_GRPC_TLS_CERT_FILE: &str = "GO_GRPC_TLS_CERT_FILE";
const GO_GRPC_TLS_KEY_FILE: &str = "GO_GRPC_TLS_KEY_FILE";
const GO_GRPC_DOMAIN_NAME: &str = "GO_GRPC_DOMAIN_NAME";

/// gRPC 連線詳細設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Grpc {
    /// 目標伺服器位址 (Host:Port)
    pub target: String,
    /// TLS 憑證檔案路徑
    pub tls_cert_file: String,
    /// TLS 私鑰檔案路徑
    pub tls_key_file: String,
    /// 網域名稱 (用於 TLS 驗證)
    pub domain_name: String,
}

const FUGLE_API_KEY: &str = "FUGLE_API_KEY";

/// Fugle 行情 API 設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Fugle {
    /// Fugle API 金鑰
    #[serde(default)]
    pub api_key: String,
}

const POSTGRESQL_HOST: &str = "POSTGRESQL_HOST";
const POSTGRESQL_PORT: &str = "POSTGRESQL_PORT";
const POSTGRESQL_USER: &str = "POSTGRESQL_USER";
const POSTGRESQL_PASSWORD: &str = "POSTGRESQL_PASSWORD";
const POSTGRESQL_DB: &str = "POSTGRESQL_DB";

// 整合測試專用的 PostgreSQL 覆蓋設定（僅在 cargo test 建置時生效）：
// 讓整合測試連往獨立的測試資料庫，避免本機 .env 指向正式庫時，
// 測試的寫入操作直接污染正式資料（2026-07-18 事件）。
// 五個變數皆未設定時完全不影響既有行為；CI 直接以 POSTGRESQL_* 指向
// 測試容器，不需要本組變數。
#[cfg(test)]
const TEST_POSTGRESQL_HOST: &str = "TEST_POSTGRESQL_HOST";
#[cfg(test)]
const TEST_POSTGRESQL_PORT: &str = "TEST_POSTGRESQL_PORT";
#[cfg(test)]
const TEST_POSTGRESQL_USER: &str = "TEST_POSTGRESQL_USER";
#[cfg(test)]
const TEST_POSTGRESQL_PASSWORD: &str = "TEST_POSTGRESQL_PASSWORD";
#[cfg(test)]
const TEST_POSTGRESQL_DB: &str = "TEST_POSTGRESQL_DB";

/// PostgreSQL 資料庫連線組態
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct PostgreSQL {
    /// 資料庫主機位址 (Host)
    #[serde(default)]
    pub host: String,
    /// 資料庫埠號 (Port)
    #[serde(default)]
    pub port: i32,
    /// 連線帳號
    #[serde(default)]
    pub user: String,
    /// 連線密碼
    #[serde(default)]
    pub password: String,
    /// 指定的資料庫名稱 (DB Name)
    #[serde(default)]
    pub db: String,
}

/// 機器人相關設定。
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Bot {
    /// Telegram 機器人設定
    pub telegram: Telegram,
}

const TELEGRAM_TOKEN: &str = "TELEGRAM_TOKEN";
const TELEGRAM_ALLOWED: &str = "TELEGRAM_ALLOWED";

/// Telegram 機器人設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Telegram {
    /// 允許存取的 User ID 及其名稱對照表 (JSON 格式儲存於環境變數)
    #[serde(default)]
    pub allowed: HashMap<i64, String>,
    /// Telegram Bot API Token
    #[serde(default)]
    pub token: String,
}

/// NoSQL 相關設定。
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct NoSQL {
    /// Redis 快取服務設定
    pub redis: Redis,
}

const REDIS_ADDR: &str = "REDIS_ADDR";
const REDIS_ACCOUNT: &str = "REDIS_ACCOUNT";
const REDIS_PASSWORD: &str = "REDIS_PASSWORD";
const REDIS_DB: &str = "REDIS_DB";
const REDIS_POOL_SIZE: &str = "REDIS_POOL_SIZE";

/// Redis 快取伺服器設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Redis {
    /// 伺服器連線位址 (Host:Port)
    pub addr: String,
    /// 使用者帳號 (如有啟用 ACL)
    pub account: String,
    /// 連線密碼
    pub password: String,
    /// 指定使用的資料庫索引 (DB Index)
    pub db: i32,
    /// 連線池大小上限；`0`（或未設定）表示使用程式預設值。
    /// 實際生效值由 `infra::nosql::redis` 收斂至安全範圍（見 `effective_pool_size`）。
    #[serde(default)]
    pub pool_size: u32,
}

/// 全域共享的組態設定項
///
/// 使用 `Lazy` 確保在第一次存取時才進行初始化，並在整個程式生命週期中共享。
///
/// 初始化時會先載入 `.env`（冪等操作，重複呼叫無害）。這很重要：
/// `app.json` 中的敏感欄位（如資料庫帳密）可能只是 placeholder，真實值靠
/// `.env` 的環境變數覆蓋。若第一個觸發 `SETTINGS` 初始化的呼叫端（例如
/// 某個沒有先呼叫 `dotenvy::dotenv()` 的測試）在 `.env` 載入前存取設定，
/// 整個 process 會鎖定 placeholder 值，之後所有資料庫連線都會失敗——
/// 且失敗與否取決於「哪個測試先跑」，非常難排查。
/// 在這裡集中載入後，初始化順序不再影響設定內容。
pub static SETTINGS: Lazy<App> = Lazy::new(|| {
    dotenvy::dotenv().ok();
    let app = App::get().expect("Config error");

    // 測試建置下優先套用 TEST_POSTGRESQL_* 覆蓋，讓整合測試與正式庫隔離。
    #[cfg(test)]
    let app = app.override_with_test_env();

    app
});

impl App {
    /// 取得系統組態項
    ///
    /// 讀取流程：
    /// 1. 嘗試讀取 `app.json` 並以 `serde_json` 解析成 [`App`]。
    /// 2. 如果 `app.json` 存在，則讀取環境變數進行覆蓋 (`override_with_env`)。
    /// 3. 如果 `app.json` 不存在，則直接從環境變數建立 (`from_env`)。
    ///
    /// 注意：`serde_json` 對型別要求嚴格——數字欄位必須是 JSON number
    /// （例如 `"port": 5432`），寫成字串（`"port": "5432"`）會在啟動時
    /// 直接解析失敗，不會被自動轉型。這是刻意的：設定檔打錯型別應該
    /// 立刻報錯，而不是默默吞掉。
    fn get() -> Result<Self> {
        let config_path = config_path();

        if config_path.exists() {
            // 直接讀檔 + serde_json 解析。兩段都可能失敗（I/O 錯誤或 JSON
            // 格式/型別錯誤），統一收斂成 anyhow::Error 交給下方 match 處理。
            let config: Result<App> = fs::read_to_string(&config_path)
                .map_err(anyhow::Error::from)
                .and_then(|text| serde_json::from_str(&text).map_err(anyhow::Error::from));

            match config {
                Ok(cfg) => return Ok(cfg.override_with_env()),
                Err(e) => {
                    // 設定檔可能包含 token、password 等敏感資訊，錯誤訊息只保留路徑與解析錯誤。
                    eprintln!("Failed to load config file {:?}: {:?}", config_path, e);
                    panic!("Failed to load config file: {:?}", e);
                }
            }
        }

        Ok(App::from_env())
    }
}

/// 回傳設定檔的路徑
fn config_path() -> PathBuf {
    PathBuf::from(CONFIG_PATH)
}

/*/// 讀取預設的設定檔
fn read_config_file() -> Result<String, io::Error> {
    let p = config_path();
    read_text_file(p)
}

/// 回傳指定路徑的文字檔的內容
pub(crate) fn read_text_file(path: PathBuf) -> Result<String, io::Error> {
    fs::read_to_string(path)
}
*/

#[cfg(test)]
mod tests;
