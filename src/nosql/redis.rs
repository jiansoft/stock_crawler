use std::sync::Arc;

use anyhow::{anyhow, Result};
use deadpool_redis::{
    redis::{cmd, AsyncCommands, ErrorKind, RedisError, RedisResult, ToRedisArgs, Value},
    Config, Connection, Pool, Runtime,
};
use futures::{stream::FuturesUnordered, StreamExt};
use once_cell::sync::Lazy;
use rust_decimal::Decimal;

use crate::{config::SETTINGS, util::text};

pub static CLIENT: Lazy<Arc<Redis>> = Lazy::new(|| Arc::new(Redis::new()));

pub struct Redis {
    pub pool: Pool,
}

impl Redis {
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
            .unwrap_or_else(|_| panic!("wrong redis URL {}", connection_url));
        pool.resize(1024);
        Redis { pool }
    }

    pub async fn ping(&self) -> Result<String> {
        let mut conn: Connection = self.pool.get().await?;
        let pong: String = cmd("PING").query_async(&mut conn).await?;

        Ok(pong)
    }

    /// Deletes a key from the Redis server.
    ///
    /// # Arguments
    ///
    /// * key: The key to be deleted from the server.
    ///
    /// # Returns
    ///
    /// * Result<()>: An empty result indicating success or an error if the deletion fails.
    pub async fn delete(&self, key: &str) -> Result<()> {
        let mut conn = self.pool.get().await?;
        conn.del::<&str, i64>(key)
            .await
            .map_err(|e| anyhow!("Failed to delete key({}) from Redis: {}", key, e))?;

        Ok(())
    }

    /// Sets a key-value pair in the Redis server with a specified time-to-live.
    ///
    /// # Type Parameters
    ///
    /// * K: The key type. It must implement ToRedisArgs.
    /// * V: The value type. It must implement ToRedisArgs.
    ///
    /// # Arguments
    ///
    /// * key: The key to be set.
    /// * value: The value to be associated with the key.
    /// * ttl_in_seconds: The time-to-live of the key-value pair in seconds.
    ///
    /// # Returns
    ///
    /// * Result<()>: An empty result indicating success or an error if the operation fails.
    pub async fn set<K: ToRedisArgs, V: ToRedisArgs>(
        &self,
        key: K,
        value: V,
        ttl_in_seconds: usize,
    ) -> Result<()> {
        let mut conn = self.pool.get().await?;

        Ok(cmd("SETEX")
            .arg(key)
            .arg(ttl_in_seconds)
            .arg(value)
            .query_async::<_>(&mut conn)
            .await?)
    }

    /// Retrieves a string value from the Redis server for the given key.
    ///
    /// # Arguments
    ///
    /// * key: The key to fetch the value for.
    ///
    /// # Returns
    ///
    /// * Result<String>: The fetched string value, or an error if the GET operation fails.
    pub async fn get_string(&self, key: &str) -> Result<String> {
        let mut conn = self.pool.get().await?;
        let value: String = cmd("GET").arg(key).query_async(&mut conn).await?;
        Ok(value)
    }

    /// Retrieves a decimal value from a data source for the given key.
    ///
    /// This method first fetches a string representation of a decimal value associated with the provided key
    /// using an asynchronous call to `get_string`. It then attempts to parse the string into a `Decimal` type.
    ///
    /// # Arguments
    ///
    /// * `key`: The key for which to fetch the decimal value.
    ///
    /// # Returns
    ///
    /// * `Result<Decimal>`: The fetched and parsed decimal value if successful, or an error if either the
    ///   fetch operation fails or the string cannot be parsed into a decimal.
    ///
    /// # Errors
    ///
    /// This method will return an error in the following situations:
    /// - If the `get_string` method call fails, the error from `get_string` will be propagated.
    /// - If the string fetched from `get_string` cannot be parsed into a `Decimal`, an error will be returned.
    pub async fn get_decimal(&self, key: &str) -> Result<Decimal> {
        let val = self.get_string(key).await?;
        text::parse_decimal(&val, None)
    }

    /// Retrieves a boolean value from the Redis server for the given key.
    ///
    /// # Arguments
    ///
    /// * key: The key to fetch the value for.
    ///
    /// # Returns
    ///
    /// * Result<bool>: The fetched boolean value, or an error if the GET operation fails.
    pub async fn get_bool(&self, key: &str) -> Result<bool> {
        let mut conn = self.pool.get().await?;
        let value: bool = cmd("GET").arg(key).query_async(&mut conn).await?;
        Ok(value)
    }

    /// Retrieves a byte array value from the Redis server for the given key.
    ///
    /// # Arguments
    ///
    /// * key: The key to fetch the value for.
    ///
    /// # Returns
    ///
    /// * Result<Vec<u8>>: The fetched byte array value, or an error if the GET operation fails or the value is not found.
    pub async fn get_bytes(&self, key: &str) -> Result<Vec<u8>> {
        let mut conn = self.pool.get().await?;
        let value: RedisResult<Value> = conn.get(key).await;
        if let Ok(Value::BulkString(data)) = value {
            return Ok(data);
        }

        if let Ok(Value::Nil) = value {
            return Err(anyhow!(
                "Cannot be found on the server using the given key."
            ));
        }

        Err(RedisError::from((ErrorKind::TypeError, "Unexpected value type")).into())
    }

    /// Retrieves keys from the Redis server that match any of the provided patterns.
    ///
    /// # Arguments
    ///
    /// * patterns: A vector of strings, each representing a pattern to match keys against.
    ///
    /// # Returns
    ///
    /// * Result<Vec<String>>: A vector of strings containing the matched keys, or an error if the operation fails.
    pub async fn get_keys(&self, patterns: Vec<String>) -> Result<Vec<String>> {
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

    /// Finds keys in the Redis server that match the provided pattern using the SCAN command.
    ///
    /// # Arguments
    ///
    /// * pattern: The pattern to match keys against.
    ///
    /// # Returns
    ///
    /// * Result<Vec<String>, Error>: A vector of strings containing the matched keys, or an error if the operation fails.
    async fn get_key(&self, pattern: String) -> Result<Vec<String>> {
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

    pub async fn contains_key(&self, pattern: &str) -> Result<bool> {
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
    use crate::{cache::SHARE, logging};
    use rust_decimal_macros::dec;

    use super::*;

    #[tokio::test]
    async fn test_redis_contains_key() {
        dotenv::dotenv().ok();

        let is_no_key_val = CLIENT.contains_key("no key").await;
        println!("no key:{:?}", is_no_key_val);
        let is_my_public_ip_val = CLIENT.contains_key("MyPublicIP").await;
        println!("MyPublicIP:{:?}", is_my_public_ip_val);
    }

    #[tokio::test]
    async fn test_redis_decimal() {
        dotenv::dotenv().ok();
        CLIENT
            .set("no key", dec!(10).to_string(), 60)
            .await
            .expect("TODO: panic message");
        let is_no_key_val = CLIENT.get_decimal("no key").await;
        println!("no_key_val_is:{:?}", is_no_key_val);
    }

    #[tokio::test]
    #[ignore]
    async fn test_redis() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 test_redis".to_string());

        //  let mut conn =   get_client().pool.get().await;
        //let client = REDIS.pool;
        println!("client.pool.status:{:?}", CLIENT.pool.status());
        println!("is_closed:{}", CLIENT.pool.is_closed());
        //let mut conn = client.pool.get().await.unwrap();

        //conn.set()
        //auth "yourpassword"
        /*  cmd("auth")
        .arg(&["0919118456"])
        .query_async::<_, ()>(&mut conn)
        .await
        .unwrap();*/
        //let _ = REDIS.set("deadpool/test_key", "43", 100).await;
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
        /*  cmd("SET")
        .arg(&["deadpool/test_key", "42"])
        .query_async::<_, ()>(&mut conn)
        .await
        .unwrap();*/

        /* let val = REDIS.get("deadpool/test_key").await;
        println!(
            "deadpool/test_key:{}",
            val.unwrap_or("Can't get".to_string())
        );

        let mut conn_1 = REDIS.pool.get().await.unwrap();
        let value: String = cmd("GET")
            .arg(&["deadpool/test_key"])
            .query_async(&mut conn_1)
            .await
            .unwrap();
        println!("value:{}", value);*/

        println!("client.pool.status:{:?}", CLIENT.pool.status());

        logging::debug_file_async("結束 test_redis".to_string());
    }
}
