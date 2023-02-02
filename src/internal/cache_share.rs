use crate::{
    internal::database::model::index,
    logging
};

use futures::executor::block_on;
use once_cell::sync::Lazy;
use std::{
    collections::HashMap,
    sync::RwLock
};


pub static CACHE_SHARE: Lazy<CacheShare> = Lazy::new(|| {
    let o = block_on(CacheShare::new());
    o
});

pub struct CacheShare {
    /// 存放台股歷年指數
    pub indices: RwLock<HashMap<String, index::Entity>>,
}

impl CacheShare {
    pub async fn new() -> Self {
        let r = index::fetch().await;
        logging::info_file_async(format!("CacheShare.indices 初始化"));
        CacheShare {
            indices: RwLock::new(r),
        }
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
    fn test_update() {
        dotenv::dotenv().ok();
        aw!(CacheShare::new());
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
