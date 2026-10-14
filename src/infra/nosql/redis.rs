use std::sync::Arc;

use deadpool_redis::{
    Config, Connection, ConnectionAddr, ConnectionInfo, CreatePoolError, Pool, RedisConnectionInfo,
    Runtime,
    redis::{AsyncCommands, RedisResult, ToRedisArgs, Value, cmd},
};
use futures::{StreamExt, stream::FuturesUnordered};
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::{core::config::SETTINGS, core::util::text};

/// 全域共享的 Redis 客戶端。
///
/// `Lazy` 靜態初始化無法回傳 `Result`，因此這裡對 [`Redis::try_new`] 的錯誤
/// 使用 panic 收尾；錯誤訊息只含 deadpool 的設定錯誤描述，不含帳號密碼。
/// 連線設定已改用結構化 `ConnectionInfo`（不經過 URL 解析），
/// 實務上 `try_new` 只剩極罕見的 pool 建置錯誤會失敗。
pub static CLIENT: Lazy<Arc<Redis>> = Lazy::new(|| {
    Arc::new(Redis::try_new().unwrap_or_else(|why| panic!("failed to create redis pool: {why}")))
});

/// Redis 操作的結構化錯誤類型。
#[derive(Debug, Error)]
pub enum RedisError {
    /// 連線池取得連線失敗。
    #[error("redis pool error: {0}")]
    Pool(#[from] deadpool_redis::PoolError),

    /// Redis 指令執行失敗。
    #[error("redis command error: {0}")]
    Command(#[from] deadpool_redis::redis::RedisError),

    /// 指定的 key 不存在。
    #[error("redis key not found")]
    NotFound,

    /// Redis 回傳了非預期的值型別。
    #[error("redis unexpected value type")]
    UnexpectedType,

    /// 值解析失敗。
    #[error("redis value parse error: {0}")]
    Parse(String),
}

/// Redis 連線池包裝器。
pub struct Redis {
    /// `deadpool-redis` 連線池。
    pub pool: Pool,
}

/// 未設定 `pool_size` 時的預設連線池大小。
///
/// 本專案的 Redis 使用情境是排程任務與盤中價格追蹤的快取讀寫，
/// 並發度遠低於先前寫死的 1024；32 已足以涵蓋尖峰（含 `get_keys`
/// 以 `FuturesUnordered` 併發 SCAN 的情況），也與 PostgreSQL 的 20 同量級。
const DEFAULT_POOL_SIZE: usize = 32;

/// 連線池大小的允許範圍：下限避免併發 SCAN 時互相餓死，上限避免尖峰連線暴衝。
const POOL_SIZE_MIN: usize = 4;
const POOL_SIZE_MAX: usize = 64;

/// 將設定值收斂為實際生效的連線池大小。
///
/// - `0`（未設定）→ 使用 [`DEFAULT_POOL_SIZE`]。
/// - 其餘值 → 收斂到 `[POOL_SIZE_MIN, POOL_SIZE_MAX]` 範圍內，
///   避免設定錯誤（例如沿用舊值 1024）造成 Redis 端連線數暴增。
fn effective_pool_size(configured: u32) -> usize {
    if configured == 0 {
        DEFAULT_POOL_SIZE
    } else {
        (configured as usize).clamp(POOL_SIZE_MIN, POOL_SIZE_MAX)
    }
}

/// 依設定組出結構化的 Redis 連線資訊。
/// 建立 Redis 連線資訊的輔助函式。
///
/// 抽成獨立純函式的原因：`try_new` 依賴全域 `SETTINGS`，把「設定 → 連線資訊」
/// 的轉換邏輯抽出來，單元測試就能直接以參數驗證各種邊界情況（含特殊字元密碼、
/// 無 port、IPv6 位址），不需要真的連上 Redis。
///
/// # 參數
/// - `addr`: Redis 主機位址，格式為 `host:port`。
/// - `account`: 認證帳號。
/// - `pw`: 認證密碼。
/// - `db`: 資料庫編號。
///
/// # 規則：
/// - `addr` 格式為 `host:port`。用 `rsplit_once` 從右邊找最後一個冒號切出 port，
///   這樣 IPv6 位址（本身含冒號，例如 `::1:6379`）也能正確處理；
///   沒有冒號或 port 不是數字時，整段視為 host 並使用 Redis 預設 port 6379。
/// - 帳號/密碼為空字串時視為「未設定」（`None`），避免對無認證的 Redis 送出空憑證。
/// - 密碼「原樣」放進結構化欄位，不經過 URL 解析，因此 `@:/#%` 等特殊字元不需要
///   percent-encoding，也不會被誤解析成 host 或 port 的一部分。
fn build_connection_info(addr: &str, account: &str, pw: &str, db: i32) -> ConnectionInfo {
    // 優先從右側切分冒號，以防 addr 為 IPv6 格式（如 ::1:6379）時誤切內部冒號
    let (host, port) = match addr.rsplit_once(':') {
        Some((host, port_str)) => match port_str.parse::<u16>() {
            Ok(port) => (host.to_string(), port),
            Err(_) => (addr.to_string(), 6379), // port 無法解析為數字時，整段當作 host 並給予預設 port
        },
        None => (addr.to_string(), 6379), // 無冒號時，整段當作 host 並給予預設 port
    };

    ConnectionInfo {
        addr: ConnectionAddr::Tcp(host, port),
        redis: RedisConnectionInfo {
            db: i64::from(db),
            // 若為空字串，則對應欄位設為 None 以免傳送空字元憑證
            username: (!account.is_empty()).then(|| account.to_string()),
            password: (!pw.is_empty()).then(|| pw.to_string()),
            ..Default::default()
        },
    }
}

impl Redis {
    /// 依照目前設定建立 Redis 連線池。
    ///
    /// 連線參數使用結構化的 [`ConnectionInfo`] 逐欄設定，而不是把帳號密碼
    /// 拼進 `redis://account:password@addr/db` 這種 URL 字串——URL 需要對
    /// 特殊字元（`@`、`:`、`/`、`#`、`%`）做 percent-encoding，直接拼字串
    /// 會讓含這些字元的強密碼被誤解析成 host 的一部分，導致啟動失敗或連錯主機。
    /// 結構化設定完全不經過 URL 解析，密碼內容原樣傳遞。
    ///
    /// # Errors
    ///
    /// 只有在 deadpool 無法依設定建立連線池時回傳錯誤（極罕見）；
    /// 注意這裡是 lazy pool，實際的 TCP 連線與認證要到第一次取連線才會發生，
    /// 健康檢查請使用 [`Self::ping`]。
    pub fn try_new() -> Result<Self, CreatePoolError> {
        let connection_info = build_connection_info(
            &SETTINGS.nosql.redis.addr,
            &SETTINGS.nosql.redis.account,
            &SETTINGS.nosql.redis.password,
            SETTINGS.nosql.redis.db,
        );

        let cfg = Config::from_connection_info(connection_info);
        let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
        // 連線池大小改由設定檔／REDIS_POOL_SIZE 決定，並收斂到安全範圍；
        // 先前寫死的 1024 遠超實際並發需求，尖峰時會對 Redis 造成不必要的連線壓力。
        pool.resize(effective_pool_size(SETTINGS.nosql.redis.pool_size));
        Ok(Redis { pool })
    }

    /// 對 Redis 執行 `PING`，確認連線可用。
    pub async fn ping(&self) -> Result<String, RedisError> {
        let mut conn: Connection = self.pool.get().await?;
        let pong: String = cmd("PING").query_async(&mut conn).await?;
        Ok(pong)
    }

    /// 刪除 Redis 中的指定 key。
    pub async fn delete(&self, key: &str) -> Result<(), RedisError> {
        let mut conn = self.pool.get().await?;
        conn.del::<&str, i64>(key).await?;
        Ok(())
    }

    /// 以 `SETEX` 寫入 key-value 並設定 TTL（秒）。
    pub async fn set<K: ToRedisArgs, V: ToRedisArgs>(
        &self,
        key: K,
        value: V,
        ttl_in_seconds: usize,
    ) -> Result<(), RedisError> {
        let mut conn = self.pool.get().await?;
        cmd("SETEX")
            .arg(key)
            .arg(ttl_in_seconds)
            .arg(value)
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// 以 `NX + EX` 方式嘗試寫入鍵值。
    ///
    /// 只有在 key 尚不存在時才會寫入，適合用在通知去重這類需要原子判斷的情境。
    ///
    /// # 回傳
    /// - `Ok(true)`：成功寫入。
    /// - `Ok(false)`：key 已存在，未寫入。
    pub async fn set_if_absent<K: ToRedisArgs, V: ToRedisArgs>(
        &self,
        key: K,
        value: V,
        ttl_in_seconds: usize,
    ) -> Result<bool, RedisError> {
        let mut conn = self.pool.get().await?;
        let result: Option<String> = cmd("SET")
            .arg(key)
            .arg(value)
            .arg("NX")
            .arg("EX")
            .arg(ttl_in_seconds)
            .query_async(&mut conn)
            .await?;
        Ok(result.is_some())
    }

    /// 僅當新價格比已記錄值「更極端」時，才以 `EX` 寫入並回報需通知。
    ///
    /// 用於「創新低（floor）或新高（ceiling）才通知」的跨重啟／跨實例節流。
    /// 透過 Lua 腳本在 Redis 端原子地完成「讀取-比較-寫入」，避免競態；
    /// 每次成功寫入都會以新價格重置 TTL。
    ///
    /// # 參數
    /// - `key`: 去重鍵值。
    /// - `value`: 目前價格。
    /// - `ttl_in_seconds`: 寫入後的存活時間（秒）。
    /// - `lower_is_more_extreme`: `true` 表示更低才算更極端（floor）；`false` 表示更高才算（ceiling）。
    ///
    /// # 回傳
    /// - `Ok(true)`：達到新的極端值（或 key 不存在），已寫入，應通知。
    /// - `Ok(false)`：未比已記錄值更極端，未寫入。
    pub async fn set_if_more_extreme(
        &self,
        key: &str,
        value: Decimal,
        ttl_in_seconds: usize,
        lower_is_more_extreme: bool,
    ) -> Result<bool, RedisError> {
        // ARGV[1]=新價格 ARGV[2]=TTL秒 ARGV[3]='1' 代表更低才更極端（floor），否則更高（ceiling）。
        const SCRIPT: &str = r"
            local cur = redis.call('GET', KEYS[1])
            local newv = tonumber(ARGV[1])
            local more_extreme
            if cur == false then
                more_extreme = true
            else
                local curv = tonumber(cur)
                if ARGV[3] == '1' then
                    more_extreme = newv < curv
                else
                    more_extreme = newv > curv
                end
            end
            if more_extreme then
                redis.call('SET', KEYS[1], ARGV[1], 'EX', ARGV[2])
                return 1
            end
            return 0
        ";

        let mut conn = self.pool.get().await?;
        let flag = if lower_is_more_extreme { "1" } else { "0" };
        let result: i64 = cmd("EVAL")
            .arg(SCRIPT)
            .arg(1)
            .arg(key)
            .arg(value.to_string())
            .arg(ttl_in_seconds)
            .arg(flag)
            .query_async(&mut conn)
            .await?;
        Ok(result == 1)
    }

    /// 取得指定 key 的字串值。
    pub async fn get_string(&self, key: &str) -> Result<String, RedisError> {
        let mut conn = self.pool.get().await?;
        let value: String = cmd("GET").arg(key).query_async(&mut conn).await?;
        Ok(value)
    }

    /// 取得指定 key 的 `Decimal` 值。
    pub async fn get_decimal(&self, key: &str) -> Result<Decimal, RedisError> {
        let val = self.get_string(key).await?;
        text::parse_decimal(&val, None).map_err(|e| RedisError::Parse(e.to_string()))
    }

    /// 取得指定 key 的布林值。
    pub async fn get_bool(&self, key: &str) -> Result<bool, RedisError> {
        let mut conn = self.pool.get().await?;
        let value: bool = cmd("GET").arg(key).query_async(&mut conn).await?;
        Ok(value)
    }

    /// 取得指定 key 的原始位元組值。
    pub async fn get_bytes(&self, key: &str) -> Result<Vec<u8>, RedisError> {
        let mut conn = self.pool.get().await?;
        let value: RedisResult<Value> = conn.get(key).await;
        match value {
            Ok(Value::BulkString(data)) => Ok(data),
            Ok(Value::Nil) => Err(RedisError::NotFound),
            _ => Err(RedisError::UnexpectedType),
        }
    }

    /// 以多個 prefix pattern 批次查詢符合的 key 清單。
    pub async fn get_keys(&self, patterns: Vec<String>) -> Result<Vec<String>, RedisError> {
        let mut results = Vec::new();
        if patterns.is_empty() {
            return Ok(results);
        }

        let mut tasks = FuturesUnordered::new();
        for pattern in patterns {
            let key = self.get_key(pattern);
            tasks.push(key);
        }

        while let Some(task_result) = tasks.next().await {
            let pattern_results = task_result?;
            results.extend(pattern_results);
        }

        Ok(results)
    }

    /// 以 SCAN 指令查詢符合 pattern 的 key 清單。
    async fn get_key(&self, pattern: String) -> Result<Vec<String>, RedisError> {
        let pool = self.pool.clone();
        let mut conn = pool.get().await?;
        let mut pattern_results = Vec::new();
        let mut cursor: isize = 0;
        loop {
            let scan_result: (isize, Vec<String>) = cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{}*", pattern))
                .query_async(&mut conn)
                .await?;

            cursor = scan_result.0;
            pattern_results.extend(scan_result.1);

            if cursor == 0 {
                break;
            }
        }

        Ok(pattern_results)
    }

    /// 以指定前綴模式確認 Redis 內是否存在任一鍵值。
    pub async fn contains_key(&self, pattern: &str) -> Result<bool, RedisError> {
        let keys = self.get_key(pattern.to_string()).await?;
        Ok(!keys.is_empty())
    }
}

#[cfg(test)]
mod tests;
