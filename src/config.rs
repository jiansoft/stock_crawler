use std::{collections::HashMap, env, path::PathBuf, str::FromStr};

use anyhow::Result;
use config::FileFormat;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::logging;

const CONFIG_PATH: &str = "app.json";

/// 應用程式總組態結構體
///
/// 此結構體整合了所有子模組的設定項目，包含資料庫、通訊協定與系統行為。
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct App {
    /// Afraid DNS 服務設定
    pub afraid: Afraid,
    /// Fugle 行情 API 設定
    pub fugle: Fugle,
    /// Dynu DNS 服務設定
    pub dyny: Dynu,
    /// No-IP DNS 服務設定
    pub noip: NoIp,
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

const AFRAID_TOKEN: &str = "AFRAID_TOKEN";
const FUGLE_API_KEY: &str = "FUGLE_API_KEY";

/// Afraid DNS 服務設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Afraid {
    /// 存取權杖 (Token)
    #[serde(default)]
    pub token: String,
    /// 伺服器網址
    #[serde(default)]
    pub url: String,
    /// 請求路徑
    #[serde(default)]
    pub path: String,
}

/// Fugle 行情 API 設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Fugle {
    /// Fugle API 金鑰
    #[serde(default)]
    pub api_key: String,
}

const DYNU_USERNAME: &str = "DYNU_USERNAME";
const DYNU_PASSWORD: &str = "DYNU_PASSWORD";

/// Dynu DNS 服務設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Dynu {
    /// 使用者名稱
    #[serde(default)]
    pub username: String,
    /// 密碼
    #[serde(default)]
    pub password: String,
}

const NOIP_USERNAME: &str = "NOIP_USERNAME";
const NOIP_PASSWORD: &str = "NOIP_PASSWORD";
const NOIP_HOSTNAMES: &str = "NOIP_HOSTNAMES";

/// No-IP DNS 服務設定
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct NoIp {
    /// 使用者名稱
    #[serde(default)]
    pub username: String,
    /// 密碼
    #[serde(default)]
    pub password: String,
    /// 關聯的網域名稱列表
    #[serde(default)]
    pub hostnames: Vec<String>,
}

const POSTGRESQL_HOST: &str = "POSTGRESQL_HOST";
const POSTGRESQL_PORT: &str = "POSTGRESQL_PORT";
const POSTGRESQL_USER: &str = "POSTGRESQL_USER";
const POSTGRESQL_PASSWORD: &str = "POSTGRESQL_PASSWORD";
const POSTGRESQL_DB: &str = "POSTGRESQL_DB";

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

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct NoSQL {
    /// Redis 快取服務設定
    pub redis: Redis,
}

const REDIS_ADDR: &str = "REDIS_ADDR";
const REDIS_ACCOUNT: &str = "REDIS_ACCOUNT";
const REDIS_PASSWORD: &str = "REDIS_PASSWORD";
const REDIS_DB: &str = "REDIS_DB";

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
}

/// 全域共享的組態設定項
///
/// 使用 `Lazy` 確保在第一次存取時才進行初始化，並在整個程式生命週期中共享。
pub static SETTINGS: Lazy<App> = Lazy::new(|| App::get().expect("Config error"));

impl App {
    /// 取得系統組態項
    ///
    /// 讀取流程：
    /// 1. 嘗試讀取並解析 `app.json`。
    /// 2. 如果 `app.json` 存在，則讀取環境變數進行覆蓋 (`override_with_env`)。
    /// 3. 如果 `app.json` 不存在，則直接從環境變數建立 (`from_env`)。
    fn get() -> Result<Self> {
        let config_path = config_path();

        if config_path.exists() {
            let config: Result<App, _> = config::Config::builder()
                .add_source(config::File::from(config_path.clone()).format(FileFormat::Json))
                .build()
                .and_then(|cfg| cfg.try_deserialize());

            match config {
                Ok(cfg) => return Ok(cfg.override_with_env()),
                Err(e) => {
                    // 列印錯誤資訊和設定檔內容
                    eprintln!(
                        "Failed to load config file: {:?}, content: {}",
                        e,
                        std::fs::read_to_string(&config_path).unwrap_or_default()
                    );
                    panic!("Failed to load config file: {:?}", e);
                }
            }
        }

        Ok(App::from_env())
    }

    /// 完全從系統環境變數中讀取所有設定值
    ///
    /// 若缺少必要的環境變數，此方法將會觸發 `expect` 導致程式崩潰。
    fn from_env() -> Self {
        let tg_allowed = env::var(TELEGRAM_ALLOWED).expect(TELEGRAM_ALLOWED);
        let mut allowed_list: HashMap<i64, String> = Default::default();
        if !tg_allowed.is_empty() {
            if let Ok(allowed) = serde_json::from_str::<HashMap<i64, String>>(&tg_allowed) {
                allowed_list = allowed;
            }
        }
        let noip_hostnames = env::var(NOIP_HOSTNAMES).expect(NOIP_HOSTNAMES);
        let mut noip_hostnames_list: Vec<String> = Default::default();

        match serde_json::from_str::<Vec<String>>(&noip_hostnames) {
            Ok(result) => {
                noip_hostnames_list = result;
            }
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to serde_json because: {:?} \r\n {}",
                    why, &noip_hostnames
                ));
            }
        }

        App {
            afraid: Afraid {
                token: env::var(AFRAID_TOKEN).expect(AFRAID_TOKEN),
                url: "".to_string(),
                path: "".to_string(),
            },
            fugle: Fugle {
                api_key: env::var(FUGLE_API_KEY).unwrap_or_default(),
            },
            postgresql: PostgreSQL {
                host: env::var(POSTGRESQL_HOST).expect(POSTGRESQL_HOST),
                port: i32::from_str(
                    &env::var(POSTGRESQL_PORT).unwrap_or_else(|_| "5432".to_string()),
                )
                .unwrap_or(5432),
                user: env::var(POSTGRESQL_USER).expect(POSTGRESQL_USER),
                password: env::var(POSTGRESQL_PASSWORD).expect(POSTGRESQL_PASSWORD),
                db: env::var(POSTGRESQL_DB).expect(POSTGRESQL_DB),
            },
            bot: Bot {
                telegram: Telegram {
                    allowed: allowed_list,
                    token: env::var(TELEGRAM_TOKEN).expect(TELEGRAM_TOKEN),
                },
            },

            nosql: NoSQL {
                redis: Redis {
                    addr: env::var(REDIS_ADDR).expect(REDIS_ADDR),
                    account: env::var(REDIS_ACCOUNT).expect(REDIS_ACCOUNT),
                    password: env::var(REDIS_PASSWORD).expect(REDIS_PASSWORD),
                    db: i32::from_str(&env::var(REDIS_DB).unwrap_or_else(|_| "6379".to_string()))
                        .unwrap_or(6379),
                },
            },

            rpc: Rpc {
                go_service: Grpc {
                    target: env::var(GO_GRPC_TARGET).expect(GO_GRPC_TARGET),
                    tls_cert_file: env::var(GO_GRPC_TLS_CERT_FILE).expect(GO_GRPC_TLS_CERT_FILE),
                    tls_key_file: env::var(GO_GRPC_TLS_KEY_FILE).expect(GO_GRPC_TLS_KEY_FILE),
                    domain_name: env::var(GO_GRPC_DOMAIN_NAME).expect(GO_GRPC_DOMAIN_NAME),
                },
            },
            system: System {
                grpc_use_port: env::var(SYSTEM_GRPC_USE_PORT)
                    .unwrap_or_else(|_| "0".to_string())
                    .parse::<i32>()
                    .unwrap_or(0),
                ssl_cert_file: env::var(SYSTEM_SSL_CERT_FILE).expect(SYSTEM_SSL_CERT_FILE),
                ssl_key_file: env::var(SYSTEM_SSL_KEY_FILE).expect(SYSTEM_SSL_KEY_FILE),
            },
            dyny: Dynu {
                username: env::var(DYNU_USERNAME).expect(DYNU_USERNAME),
                password: env::var(DYNU_PASSWORD).expect(DYNU_PASSWORD),
            },
            noip: NoIp {
                username: env::var(NOIP_USERNAME).expect(NOIP_USERNAME),
                password: env::var(NOIP_USERNAME).expect(NOIP_USERNAME),
                hostnames: noip_hostnames_list,
            },
        }
    }

    /// 將來至於 env 的設定值覆蓋掉 json 上的設定值
    fn override_with_env(mut self) -> Self {
        if let Ok(token) = env::var(AFRAID_TOKEN) {
            self.afraid.token = token;
        }

        if let Ok(api_key) = env::var(FUGLE_API_KEY) {
            self.fugle.api_key = api_key;
        }

        if let Ok(username) = env::var(DYNU_USERNAME) {
            self.dyny.username = username;
        }

        if let Ok(pw) = env::var(DYNU_PASSWORD) {
            self.dyny.password = pw;
        }

        if let Ok(username) = env::var(NOIP_USERNAME) {
            self.noip.username = username;
        }

        if let Ok(pw) = env::var(NOIP_PASSWORD) {
            self.noip.password = pw;
        }

        if let Ok(hostnames) = env::var(NOIP_HOSTNAMES) {
            match serde_json::from_str::<Vec<String>>(&hostnames) {
                Ok(result) => {
                    self.noip.hostnames = result;
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to serde_json because: {:?} \r\n {}",
                        why, &hostnames
                    ));
                }
            }
        }

        if let Ok(cert_file) = env::var(SYSTEM_SSL_CERT_FILE) {
            self.system.ssl_cert_file = cert_file;
        }
        if let Ok(key_file) = env::var(SYSTEM_SSL_KEY_FILE) {
            self.system.ssl_key_file = key_file;
        }

        if let Ok(target) = env::var(GO_GRPC_TARGET) {
            self.rpc.go_service.target = target;
        }

        if let Ok(cert) = env::var(GO_GRPC_TLS_CERT_FILE) {
            self.rpc.go_service.tls_cert_file = cert;
        }

        if let Ok(key) = env::var(GO_GRPC_TLS_KEY_FILE) {
            self.rpc.go_service.tls_key_file = key;
        }

        if let Ok(domain_name) = env::var(GO_GRPC_DOMAIN_NAME) {
            self.rpc.go_service.domain_name = domain_name;
        }

        if let Ok(host) = env::var(POSTGRESQL_HOST) {
            self.postgresql.host = host;
        }

        if let Ok(port) = env::var(POSTGRESQL_PORT) {
            self.postgresql.port = i32::from_str(&port).unwrap_or(5432);
        }

        if let Ok(user) = env::var(POSTGRESQL_USER) {
            self.postgresql.user = user;
        }

        if let Ok(password) = env::var(POSTGRESQL_PASSWORD) {
            self.postgresql.password = password;
        }

        if let Ok(db) = env::var(POSTGRESQL_DB) {
            self.postgresql.db = db;
        }

        if let Ok(tg_allowed) = env::var(TELEGRAM_ALLOWED) {
            match serde_json::from_str::<HashMap<i64, String>>(&tg_allowed) {
                Ok(allowed) => {
                    self.bot.telegram.allowed = allowed;
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to serde_json because: {:?} \r\n {}",
                        why, &tg_allowed
                    ));
                }
            }
        }

        if let Ok(token) = env::var(TELEGRAM_TOKEN) {
            self.bot.telegram.token = token
        }

        if let Ok(addr) = env::var(REDIS_ADDR) {
            self.nosql.redis.addr = addr
        }

        if let Ok(db) = env::var(REDIS_DB) {
            self.nosql.redis.db = i32::from_str(db.as_str()).unwrap_or(6379)
        }

        if let Ok(account) = env::var(REDIS_ACCOUNT) {
            self.nosql.redis.account = account
        }

        if let Ok(password) = env::var(REDIS_PASSWORD) {
            self.nosql.redis.password = password
        }

        self
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
mod tests {
    use std::time;

    use super::*;

    #[tokio::test]
    async fn test_init() {
        dotenv::dotenv().ok();
        logging::debug_file_async(format!("SETTINGS.system: {:#?}\r\n", SETTINGS.system));
        logging::debug_file_async(format!(
            "SETTINGS.postgresql: {:#?}\r\nSETTINGS.secret: {:#?}\r\n",
            SETTINGS.postgresql, SETTINGS.bot
        ));

        logging::debug_file_async(format!(
            "SETTINGS.nosql.redis: {:#?}\r\n",
            SETTINGS.nosql.redis
        ));

        logging::debug_file_async(format!("SETTINGS.rpc: {:#?}\r\n", SETTINGS.rpc));

        let mut map: HashMap<i64, String> = HashMap::new();
        map.insert(123, "QQ".to_string());
        map.insert(456, "QQ".to_string());
        let json_str = serde_json::to_string(&map).expect("TODO: panic message");

        logging::debug_file_async(format!("serde_json: {}\r\n", &json_str));
        match serde_json::from_str::<HashMap<i64, String>>(&json_str) {
            Ok(json) => {
                logging::debug_file_async(format!("json: {:?}\r\n", json));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to serde_json because: {:?} \r\n {}",
                    why, &json_str
                ));
            }
        }
        tokio::time::sleep(time::Duration::from_secs(1)).await;
    }
}
