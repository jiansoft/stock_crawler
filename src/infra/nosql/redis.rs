use std::sync::Arc;

use deadpool_redis::{
    Config, Connection, Pool, Runtime,
    redis::{AsyncCommands, RedisResult, ToRedisArgs, Value, cmd},
};
use futures::{StreamExt, stream::FuturesUnordered};
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::{core::config::SETTINGS, core::util::text};

/// 全域共享的 Redis 客戶端。
pub static CLIENT: Lazy<Arc<Redis>> = Lazy::new(|| Arc::new(Redis::new()));

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

impl Redis {
    /// 依照目前設定建立 Redis 連線池。
    pub fn new() -> Self {
        //redis://mypassword@127.0.0.1:6379
        let connection_url = format!(
            "redis://{}:{}@{}/{}",
            SETTINGS.nosql.redis.account,
            SETTINGS.nosql.redis.password,
            SETTINGS.nosql.redis.addr,
            SETTINGS.nosql.redis.db
        );

        let cfg = Config::from_url(&connection_url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .unwrap_or_else(|_| panic!("wrong redis URL"));
        pool.resize(1024);
        Redis { pool }
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

impl Default for Redis {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;
    use rust_decimal_macros::dec;

    use super::*;

    async fn skip_when_redis_unavailable() -> bool {
        if CLIENT.ping().await.is_ok() {
            return false;
        }

        println!("skip redis test: Redis is unavailable in current environment");
        true
    }

    /// 驗證 `contains_key` 的基本查詢流程。
    #[tokio::test]
    async fn test_redis_contains_key() {
        dotenvy::dotenv().ok();
        if skip_when_redis_unavailable().await {
            return;
        }

        let is_no_key_val = CLIENT.contains_key("no key").await;
        println!("no key:{:?}", is_no_key_val);
        let is_my_public_ip_val = CLIENT.contains_key("MyPublicIP").await;
        println!("MyPublicIP:{:?}", is_my_public_ip_val);
    }

    /// 驗證 decimal 存取流程。
    #[tokio::test]
    async fn test_redis_decimal() {
        dotenvy::dotenv().ok();
        if skip_when_redis_unavailable().await {
            return;
        }
        CLIENT
            .set("no key", dec!(10).to_string(), 60)
            .await
            .expect("TODO: panic message");
        let is_no_key_val = CLIENT.get_decimal("no key").await;
        println!("no_key_val_is:{:?}", is_no_key_val);
    }

    /// 驗證「創新低才寫入」的去重邏輯。
    #[tokio::test]
    async fn test_set_if_more_extreme_floor() {
        dotenvy::dotenv().ok();
        if skip_when_redis_unavailable().await {
            return;
        }

        let key = "deadpool/test_more_extreme_floor";
        let _ = CLIENT.delete(key).await;

        // 首次：key 不存在 → 寫入並通知。
        assert!(
            CLIENT
                .set_if_more_extreme(key, dec!(86.0), 60, true)
                .await
                .unwrap()
        );
        // 較高價：非新低 → 不寫入。
        assert!(
            !CLIENT
                .set_if_more_extreme(key, dec!(86.1), 60, true)
                .await
                .unwrap()
        );
        // 創新低 → 寫入並通知。
        assert!(
            CLIENT
                .set_if_more_extreme(key, dec!(85.9), 60, true)
                .await
                .unwrap()
        );
        // 回升：非新低 → 不寫入。
        assert!(
            !CLIENT
                .set_if_more_extreme(key, dec!(86.0), 60, true)
                .await
                .unwrap()
        );

        let _ = CLIENT.delete(key).await;
    }

    /// 驗證「創新高才寫入」的去重邏輯。
    #[tokio::test]
    async fn test_set_if_more_extreme_ceiling() {
        dotenvy::dotenv().ok();
        if skip_when_redis_unavailable().await {
            return;
        }

        let key = "deadpool/test_more_extreme_ceiling";
        let _ = CLIENT.delete(key).await;

        assert!(
            CLIENT
                .set_if_more_extreme(key, dec!(100.0), 60, false)
                .await
                .unwrap()
        );
        assert!(
            !CLIENT
                .set_if_more_extreme(key, dec!(99.9), 60, false)
                .await
                .unwrap()
        );
        assert!(
            CLIENT
                .set_if_more_extreme(key, dec!(100.1), 60, false)
                .await
                .unwrap()
        );

        let _ = CLIENT.delete(key).await;
    }

    /// 驗證 Redis 常用操作。
    #[tokio::test]
    async fn test_redis() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 test_redis");

        println!("client.pool.status:{:?}", CLIENT.pool.status());
        println!("is_closed:{}", CLIENT.pool.is_closed());

        let is_no_key_val = CLIENT.get_string("no key").await;
        match is_no_key_val {
            Ok(_) => {}
            Err(why) => {
                println!("is_no_key_val err {:#?}", why);
            }
        }

        let key = "deadpool/test_key";
        let _ = CLIENT.set(key, true, 100).await;
        let bool_val = CLIENT.get_bool(key).await;
        assert!(
            matches!(bool_val, Ok(true)),
            "Expected {} but got {:#?}",
            true,
            bool_val
        );

        println!("bool_val:{}", bool_val.unwrap());

        let vec_val = CLIENT.get_bytes(key).await;
        println!("vec_val:{:#?}", vec_val);

        let _ = CLIENT.delete(key).await;
        let bool_val = CLIENT.get_bool(key).await;
        println!("bool_val:{:#?}", bool_val);

        let _ = CLIENT.set(key, "中文", 100).await;
        let string_val = CLIENT.get_string(key).await;

        if let Ok(val1) = string_val {
            assert_eq!(val1, "中文".to_string());
            println!("string_val:{}", val1);
        }
        let get_all_keys = CLIENT
            .get_keys(vec![
                "YieldEstimate".to_string(),
                "InventoryProfitReport".to_string(),
                "Revenues".to_string(),
            ])
            .await;
        println!("get_all_keys:{:#?}", get_all_keys);

        println!("client.pool.status:{:?}", CLIENT.pool.status());

        tracing::debug!("結束 test_redis");
    }
}
