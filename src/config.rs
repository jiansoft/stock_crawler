use std::{collections::HashMap, env, fs, io, path::PathBuf, str::FromStr, u8};

use anyhow::Result;
use config::{Config as config_config, File as config_file};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::logging;

const CONFIG_PATH: &str = "app.json";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct App {
    pub afraid: Afraid,
    pub dyny: Dynu,
    pub bot: Bot,
    pub postgresql: PostgreSQL,
    pub rpc: Rpc,
    pub nosql: NoSQL,
    pub system: System,
}

const SYSTEM_GRPC_USE_PORT: &str = "SYSTEM_GRPC_USE_PORT";
const SYSTEM_SSL_CERT_FILE: &str = "SYSTEM_SSL_CERT_FILE";
const SYSTEM_SSL_KEY_FILE: &str = "SYSTEM_SSL_KEY_FILE";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct System {
    pub grpc_use_port: i32,
    pub ssl_cert_file: String,
    pub ssl_key_file: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Rpc {
    pub go_service: Grpc,
}

const GO_GRPC_TARGET: &str = "GO_GRPC_TARGET";
const GO_GRPC_TLS_CERT_FILE: &str = "GO_GRPC_TLS_CERT_FILE";
const GO_GRPC_TLS_KEY_FILE: &str = "GO_GRPC_TLS_KEY_FILE";
const GO_GRPC_DOMAIN_NAME: &str = "GO_GRPC_DOMAIN_NAME";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Grpc {
    pub target: String,
    pub tls_cert_file: String,
    pub tls_key_file: String,
    pub domain_name: String,
}

const AFRAID_TOKEN: &str = "AFRAID_TOKEN";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Afraid {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub path: String,
}

const DYNU_USERNAME: &str = "DYNU_USERNAME";
const DYNU_PASSWORD: &str = "DYNU_PASSWORD";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Dynu {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

const POSTGRESQL_HOST: &str = "POSTGRESQL_HOST";
const POSTGRESQL_PORT: &str = "POSTGRESQL_PORT";
const POSTGRESQL_USER: &str = "POSTGRESQL_USER";
const POSTGRESQL_PASSWORD: &str = "POSTGRESQL_PASSWORD";
const POSTGRESQL_DB: &str = "POSTGRESQL_DB";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct PostgreSQL {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: i32,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub db: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Bot {
    pub telegram: Telegram,
}

const TELEGRAM_TOKEN: &str = "TELEGRAM_TOKEN";
const TELEGRAM_ALLOWED: &str = "TELEGRAM_ALLOWED";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Telegram {
    pub allowed: HashMap<i64, String>,
    pub token: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct NoSQL {
    pub redis: Redis,
}

const REDIS_ADDR: &str = "REDIS_ADDR";
const REDIS_ACCOUNT: &str = "REDIS_ACCOUNT";
const REDIS_PASSWORD: &str = "REDIS_PASSWORD";
const REDIS_DB: &str = "REDIS_DB";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Redis {
    pub addr: String,
    pub account: String,
    pub password: String,
    pub db: i32,
}

pub static SETTINGS: Lazy<App> = Lazy::new(|| App::get().expect("Config error"));

impl App {
    pub fn new() -> Self {
        //讀取設定檔
        let config_txt = read_config_file();
        //取得文字檔的內容
        let text_content = config_txt.unwrap_or_else(|_| Default::default());

        if text_content.is_empty() {
            return Default::default();
        }

        //轉成Config 物件
        let from_json = serde_json::from_str::<App>(text_content.as_str());
        match from_json {
            Err(why) => {
                logging::error_file_async(format!(
                    "I can't read the config context because {:?}",
                    why
                ));
                Default::default()
            }
            Ok(_config) => _config.override_with_env(),
        }
    }

    fn get() -> Result<Self> {
        let config_path = config_path();
        if config_path.exists() {
            let config: App = config_config::builder()
                .add_source(config_file::from(config_path))
                .build()?
                .try_deserialize()?;
            return Ok(config.override_with_env());
        }

        Ok(App::from_env())
    }

    /// 從 env 中讀取設定值
    fn from_env() -> Self {
        let tg_allowed = env::var(TELEGRAM_ALLOWED).expect(TELEGRAM_ALLOWED);
        let mut allowed_list: HashMap<i64, String> = Default::default();
        if !tg_allowed.is_empty() {
            if let Ok(allowed) = serde_json::from_str::<HashMap<i64, String>>(&tg_allowed) {
                allowed_list = allowed;
            }
        }

        App {
            afraid: Afraid {
                token: env::var(AFRAID_TOKEN).expect(AFRAID_TOKEN),
                url: "".to_string(),
                path: "".to_string(),
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
        }
    }

    /// 將來至於 env 的設定值覆蓋掉 json 上的設定值
    fn override_with_env(mut self) -> Self {
        if let Ok(token) = env::var(AFRAID_TOKEN) {
            self.afraid.token = token;
        }

        if let Ok(username) = env::var(DYNU_USERNAME) {
            self.dyny.username = username;
        }

        if let Ok(pw) = env::var(DYNU_PASSWORD) {
            self.dyny.password = pw;
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

/// 讀取預設的設定檔
fn read_config_file() -> Result<String, io::Error> {
    let p = config_path();
    read_text_file(p)
}

/// 回傳指定路徑的文字檔的內容
pub(crate) fn read_text_file(path: PathBuf) -> Result<String, io::Error> {
    fs::read_to_string(path)
}

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
