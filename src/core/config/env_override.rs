//! 從環境變數建立或覆蓋組態：沒有 `app.json` 時全部讀環境變數，有的話以環境變數覆蓋檔案中的值；
//! 測試建置另以 `TEST_POSTGRESQL_*` 覆蓋資料庫連線。環境變數名稱的常數與各結構定義放在父模組。

use std::env;

use super::*;

impl App {
    /// 完全從系統環境變數中讀取所有設定值
    ///
    /// 若缺少必要的環境變數，此方法將會觸發 `expect` 導致程式崩潰。
    pub(super) fn from_env() -> Self {
        Self::from_lookup(&|name| env::var(name).ok())
    }

    /// [`Self::from_env`] 的本體：以 `lookup` 取得各設定值（測試時可換成固定的對照表）。
    ///
    /// 缺少必要的值時以該名稱 panic，與直接讀環境變數的行為相同。
    pub(super) fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Self {
        // 讀取 Telegram 允許的用戶清單，並解析成 JSON 格式
        let tg_allowed = lookup(TELEGRAM_ALLOWED).expect(TELEGRAM_ALLOWED);
        let mut allowed_list: HashMap<i64, String> = Default::default();
        if !tg_allowed.is_empty()
            && let Ok(allowed) = serde_json::from_str::<HashMap<i64, String>>(&tg_allowed)
        {
            allowed_list = allowed;
        }

        App {
            fugle: Fugle {
                // 若 Fugle API Key 不存在則使用預設的空字串
                api_key: lookup(FUGLE_API_KEY).unwrap_or_default(),
            },
            postgresql: PostgreSQL {
                // 從環境變數讀取資料庫連線主機
                host: lookup(POSTGRESQL_HOST).expect(POSTGRESQL_HOST),
                // 讀取資料庫埠號，預設為 5432
                port: i32::from_str(&lookup(POSTGRESQL_PORT).unwrap_or_else(|| "5432".to_string()))
                    .unwrap_or(5432),
                // 讀取資料庫帳號
                user: lookup(POSTGRESQL_USER).expect(POSTGRESQL_USER),
                // 讀取資料庫密碼
                password: lookup(POSTGRESQL_PASSWORD).expect(POSTGRESQL_PASSWORD),
                // 讀取資料庫名稱
                db: lookup(POSTGRESQL_DB).expect(POSTGRESQL_DB),
            },
            bot: Bot {
                telegram: Telegram {
                    allowed: allowed_list,
                    // 讀取 Telegram 機器人 Token
                    token: lookup(TELEGRAM_TOKEN).expect(TELEGRAM_TOKEN),
                },
            },

            nosql: NoSQL {
                redis: Redis {
                    // 讀取 Redis 連線位址
                    addr: lookup(REDIS_ADDR).expect(REDIS_ADDR),
                    // 讀取 Redis 連線帳號
                    account: lookup(REDIS_ACCOUNT).expect(REDIS_ACCOUNT),
                    // 讀取 Redis 連線密碼
                    password: lookup(REDIS_PASSWORD).expect(REDIS_PASSWORD),
                    // 讀取 Redis 資料庫索引，預設為 6379
                    db: i32::from_str(&lookup(REDIS_DB).unwrap_or_else(|| "6379".to_string()))
                        .unwrap_or(6379),
                    // 讀取 Redis 連線池大小；未設定或解析失敗時為 0（使用程式預設值）
                    pool_size: lookup(REDIS_POOL_SIZE)
                        .and_then(|v| u32::from_str(&v).ok())
                        .unwrap_or(0),
                },
            },

            rpc: Rpc {
                go_service: Grpc {
                    // 讀取 Go gRPC 連線目標
                    target: lookup(GO_GRPC_TARGET).expect(GO_GRPC_TARGET),
                    // 讀取 TLS 憑證檔案路徑
                    tls_cert_file: lookup(GO_GRPC_TLS_CERT_FILE).expect(GO_GRPC_TLS_CERT_FILE),
                    // 讀取 TLS 金鑰檔案路徑
                    tls_key_file: lookup(GO_GRPC_TLS_KEY_FILE).expect(GO_GRPC_TLS_KEY_FILE),
                    // 讀取 TLS 憑證的網域名稱
                    domain_name: lookup(GO_GRPC_DOMAIN_NAME).expect(GO_GRPC_DOMAIN_NAME),
                },
            },
            system: System {
                // 讀取 gRPC 服務埠號，預設為 0
                grpc_use_port: lookup(SYSTEM_GRPC_USE_PORT)
                    .unwrap_or_else(|| "0".to_string())
                    .parse::<i32>()
                    .unwrap_or(0),
                // 讀取 SSL 憑證路徑
                ssl_cert_file: lookup(SYSTEM_SSL_CERT_FILE).expect(SYSTEM_SSL_CERT_FILE),
                // 讀取 SSL 金鑰路徑
                ssl_key_file: lookup(SYSTEM_SSL_KEY_FILE).expect(SYSTEM_SSL_KEY_FILE),
            },
            logging: Logging {
                file: FileLogging {
                    // 讀取單檔大小上限（MB）；未設定或解析失敗時為 0（使用程式預設值）
                    max_size_mb: lookup(LOG_FILE_MAX_SIZE_MB)
                        .and_then(|v| u64::from_str(v.trim()).ok())
                        .unwrap_or(0),
                    // 讀取日誌保留天數；未設定或解析失敗時為 0（使用程式預設值）
                    max_age_days: lookup(LOG_FILE_MAX_AGE_DAYS)
                        .and_then(|v| i64::from_str(v.trim()).ok())
                        .unwrap_or(0),
                },
                seq: SeqLogging {
                    // 讀取 Seq 伺服器網址
                    server_url: lookup(SEQ_SERVER_URL).unwrap_or_default(),
                    // 讀取 Seq API 金鑰
                    api_key: lookup(SEQ_API_KEY).unwrap_or_default(),
                },
            },
        }
    }

    /// 測試建置時以 `TEST_POSTGRESQL_*` 覆蓋 PostgreSQL 連線設定。
    ///
    /// 僅覆蓋有設定的欄位；全部未設定時不改變任何行為。
    /// 目的：本機 `.env` 的 `POSTGRESQL_*` 可能指向正式庫，
    /// 整合測試應改連獨立的測試資料庫以免寫入污染正式資料。
    #[cfg(test)]
    pub(super) fn override_with_test_env(self) -> Self {
        self.override_with_test(&|name| env::var(name).ok())
    }

    /// [`Self::override_with_test_env`] 的本體：以 `lookup` 取得各設定值。
    #[cfg(test)]
    pub(super) fn override_with_test(mut self, lookup: &dyn Fn(&str) -> Option<String>) -> Self {
        if let Some(host) = lookup(TEST_POSTGRESQL_HOST) {
            self.postgresql.host = host;
        }
        if let Some(port) = lookup(TEST_POSTGRESQL_PORT) {
            self.postgresql.port = i32::from_str(&port).unwrap_or(5432);
        }
        if let Some(user) = lookup(TEST_POSTGRESQL_USER) {
            self.postgresql.user = user;
        }
        if let Some(password) = lookup(TEST_POSTGRESQL_PASSWORD) {
            self.postgresql.password = password;
        }
        if let Some(db) = lookup(TEST_POSTGRESQL_DB) {
            self.postgresql.db = db;
        }

        self
    }

    /// 以環境變數覆蓋設定檔中的值。
    ///
    /// 這裡會逐一檢查並使用環境變數覆蓋已從檔案中載入的設定。
    pub(super) fn override_with_env(self) -> Self {
        self.override_with(&|name| env::var(name).ok())
    }

    /// [`Self::override_with_env`] 的本體：以 `lookup` 取得各設定值（測試時可換成固定的對照表）。
    pub(super) fn override_with(mut self, lookup: &dyn Fn(&str) -> Option<String>) -> Self {
        // 若環境變數中有 Fugle 行情金鑰，則覆蓋設定
        if let Some(api_key) = lookup(FUGLE_API_KEY) {
            self.fugle.api_key = api_key;
        }

        // 若環境變數中有 SSL 憑證資訊，則覆蓋設定
        if let Some(cert_file) = lookup(SYSTEM_SSL_CERT_FILE) {
            self.system.ssl_cert_file = cert_file;
        }
        if let Some(key_file) = lookup(SYSTEM_SSL_KEY_FILE) {
            self.system.ssl_key_file = key_file;
        }

        // 若環境變數中有 Seq 日誌收集伺服器資訊，則覆蓋設定
        if let Some(server_url) = lookup(SEQ_SERVER_URL) {
            self.logging.seq.server_url = server_url;
        }
        if let Some(api_key) = lookup(SEQ_API_KEY) {
            self.logging.seq.api_key = api_key;
        }

        // 若環境變數中有輪轉日誌檔設定，則覆蓋設定；
        // 解析失敗（例如打成非數字）時保留 app.json 的值，避免靜默歸零。
        if let Some(max_size_mb) =
            lookup(LOG_FILE_MAX_SIZE_MB).and_then(|v| u64::from_str(v.trim()).ok())
        {
            self.logging.file.max_size_mb = max_size_mb;
        }
        if let Some(max_age_days) =
            lookup(LOG_FILE_MAX_AGE_DAYS).and_then(|v| i64::from_str(v.trim()).ok())
        {
            self.logging.file.max_age_days = max_age_days;
        }

        // 若環境變數中有 Go 後端 gRPC 服務資訊，則覆蓋設定
        if let Some(target) = lookup(GO_GRPC_TARGET) {
            self.rpc.go_service.target = target;
        }
        if let Some(cert) = lookup(GO_GRPC_TLS_CERT_FILE) {
            self.rpc.go_service.tls_cert_file = cert;
        }
        if let Some(key) = lookup(GO_GRPC_TLS_KEY_FILE) {
            self.rpc.go_service.tls_key_file = key;
        }
        if let Some(domain_name) = lookup(GO_GRPC_DOMAIN_NAME) {
            self.rpc.go_service.domain_name = domain_name;
        }

        // 若環境變數中有 PostgreSQL 連線資訊，則覆蓋設定
        if let Some(host) = lookup(POSTGRESQL_HOST) {
            self.postgresql.host = host;
        }
        if let Some(port) = lookup(POSTGRESQL_PORT) {
            self.postgresql.port = i32::from_str(&port).unwrap_or(5432);
        }
        if let Some(user) = lookup(POSTGRESQL_USER) {
            self.postgresql.user = user;
        }
        if let Some(password) = lookup(POSTGRESQL_PASSWORD) {
            self.postgresql.password = password;
        }
        if let Some(db) = lookup(POSTGRESQL_DB) {
            self.postgresql.db = db;
        }

        // 若環境變數中有 Telegram 允許名單與金鑰，則覆蓋設定
        if let Some(tg_allowed) = lookup(TELEGRAM_ALLOWED) {
            match serde_json::from_str::<HashMap<i64, String>>(&tg_allowed) {
                Ok(allowed) => {
                    self.bot.telegram.allowed = allowed;
                }
                Err(why) => {
                    // 注意：這行 tracing 位於 SETTINGS 的 Lazy 初始化路徑內，會經
                    // FileLogLayer 觸發 core::logging::LOGGER 的初始化。因此 logging
                    // 那側不得反向讀取 SETTINGS，否則會形成同執行緒的 Lazy 重入而死鎖
                    // （詳見 core::logging::Logger::create_writer 的說明）。
                    tracing::error!(
                        "Failed to serde_json because: {:?} \r\n {}",
                        why,
                        &tg_allowed
                    );
                }
            }
        }
        if let Some(token) = lookup(TELEGRAM_TOKEN) {
            self.bot.telegram.token = token
        }

        // 若環境變數中有 Redis 快取連線資訊，則覆蓋設定
        if let Some(addr) = lookup(REDIS_ADDR) {
            self.nosql.redis.addr = addr
        }
        if let Some(db) = lookup(REDIS_DB) {
            self.nosql.redis.db = i32::from_str(db.as_str()).unwrap_or(6379)
        }
        if let Some(account) = lookup(REDIS_ACCOUNT) {
            self.nosql.redis.account = account
        }
        if let Some(password) = lookup(REDIS_PASSWORD) {
            self.nosql.redis.password = password
        }
        if let Some(pool_size) = lookup(REDIS_POOL_SIZE) {
            // 解析失敗時回退為 0（使用程式預設值），而不是沿用 app.json 的值，
            // 讓「環境變數有設定」的優先權語意與其他欄位一致。
            self.nosql.redis.pool_size = u32::from_str(&pool_size).unwrap_or(0)
        }

        self
    }
}
