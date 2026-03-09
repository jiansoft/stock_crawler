use std::collections::HashMap;

/// 可轉成快取鍵值的型別介面。
pub trait Keyable {
    /// 回傳不含前綴的主要鍵值。
    fn key(&self) -> String;
    /// 回傳含前綴的完整鍵值。
    fn key_with_prefix(&self) -> String;
    // 後置字元
    // fn key_with_suffix(&self) -> String;
}

/// 將一組可鍵值化的實體轉成 `HashMap`。
pub fn vec_to_hashmap<T: Keyable>(entities: Vec<T>) -> HashMap<String, T> {
    let mut map = HashMap::with_capacity(entities.len());
    for e in entities {
        map.insert(e.key(), e);
    }
    map
}
