use crate::{internal::database::model::index, logging};
//use futures::executor::block_on;
use once_cell::sync::Lazy;
use std::{collections::HashMap, sync::RwLock};

pub static CACHE_SHARE: Lazy<CacheShare> = Lazy::new(CacheShare::new);

pub struct CacheShare {
    /// 存放台股歷年指數
    pub indices: RwLock<HashMap<String, index::Entity>>,
}

impl CacheShare {
    pub fn new() -> Self {
        CacheShare {
            indices: RwLock::new(HashMap::new()),
        }
    }

    pub async fn load(&self) -> Option<()> {
        let indices = index::fetch().await;
        match self.indices.write() {
            Ok(mut i) => {
                i.extend(indices.iter().map(|(k, v)| (k.clone(), v.clone())));
                logging::info_file_async(format!("CacheShare.indices 初始化 {}", i.len()));
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
            }
        }

        Some(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn test_init() {
        dotenv::dotenv().ok();
        let _ = CACHE_SHARE.indices.read().is_ok();
        // CACHE_SHARE.init().await;
    }

    macro_rules! aw {
        ($e:expr) => {
            tokio_test::block_on($e)
        };
    }

    #[test]
    fn test_load() {
        dotenv::dotenv().ok();

        aw!(CACHE_SHARE.load());
        //aw!(CacheShare::new());
        /*aw!(async{
             for e in CACHE_SHARE.indices.read().unwrap().iter() {
                logging::info_file_async(format!(
                    "test_update indices e.date {:?} e.index {:?}",
                    e.1.date, e.1.index
                ));
            }
        });*/
    }
}
