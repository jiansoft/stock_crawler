#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 原表名 stock_exchange_market
pub struct Entity {
    pub stock_exchange_market_id: i32,
    pub stock_exchange_id: i32,
    pub code: String,
    pub name: String,
}

impl Entity {
    pub fn new(
        stock_exchange_market_id: i32,
        stock_exchange_id: i32,
    ) -> Self {
        Entity {
            stock_exchange_market_id,
            stock_exchange_id,
            code: "".to_string(),
            name: "".to_string(),
        }
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Self {
            stock_exchange_market_id: self.stock_exchange_market_id,
            stock_exchange_id: self.stock_exchange_id,
            code: self.code.to_string(),
            name: self.name.to_string(),
        }
    }
}
