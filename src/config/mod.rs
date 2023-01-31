extern crate serde;
extern crate serde_json;

use crate::logging;
use config::{Config as config_config, File as config_file};
use lazy_static::lazy_static;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::{env, fs, io, path::PathBuf};

const CONFIG_PATH: &str = "app.json";

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct App {
    #[serde(default)]
    pub afraid: Afraid,
    pub postgresql: PostgreSQL,
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

//第一種 lazy 的作法
lazy_static! {
    pub static ref DEFAULT: App = App::new();
}

//第二種 lazy 的作法
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
        return match from_json {
            Err(why) => {
                logging::error_file_async(format!(
                    "I can't read the config context because {:?}",
                    why
                ));
                Default::default()
            }
            Ok(_config) => _config.override_with_env(),
        };
    }

    fn get() -> Result<Self, config::ConfigError> {
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
        App {
            afraid: Afraid {
                token: env::var(AFRAID_TOKEN).expect(AFRAID_TOKEN),
                url: "".to_string(),
                path: "".to_string(),
            },
            postgresql: PostgreSQL {
                host: env::var(POSTGRESQL_HOST).expect(POSTGRESQL_HOST),
                port: i32::from_str(&*env::var(POSTGRESQL_PORT).unwrap_or("5432".to_string()))
                    .unwrap_or(5432),
                user: env::var(POSTGRESQL_USER).expect(POSTGRESQL_USER),
                password: env::var(POSTGRESQL_PASSWORD).expect(POSTGRESQL_PASSWORD),
                db: env::var(POSTGRESQL_DB).expect(POSTGRESQL_DB),
            },
        }
    }

    /// 將來至於 env 的設定值覆蓋掉 json 上的設定值
    fn override_with_env(mut self) -> Self {
        if let Ok(token) = env::var(AFRAID_TOKEN) {
            self.afraid.token = token;
        }

        if let Ok(host) = env::var(POSTGRESQL_HOST) {
            self.postgresql.host = host;
        }

        if let Ok(port) = env::var(POSTGRESQL_PORT) {
            self.postgresql.port = i32::from_str(&*port).unwrap_or(5432);
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
