use std::collections::HashMap;

/// 股息記錄的鍵名
pub trait Keyable {
    fn key(&self) -> String;
}

pub fn vec_to_hashmap<T: Keyable>(entities: Vec<T>) -> HashMap<String, T> {
    let mut map = HashMap::new();
    for e in entities {
        map.insert(e.key(), e);
    }
    map
}
