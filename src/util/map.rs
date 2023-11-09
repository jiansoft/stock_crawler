use std::collections::HashMap;

/// 股息記錄的鍵名
pub trait Keyable {
    fn key(&self) -> String;
    /// 含前置字元
    fn key_with_prefix(&self) -> String;
    // 後置字元
    // fn key_with_suffix(&self) -> String;
}

pub fn vec_to_hashmap<T: Keyable>(entities: Vec<T>) -> HashMap<String, T> {
    let mut map = HashMap::with_capacity(entities.len());
    for e in entities {
        map.insert(e.key(), e);
    }
    map
}
