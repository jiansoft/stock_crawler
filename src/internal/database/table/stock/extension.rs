use sqlx::FromRow;

#[derive(FromRow, Debug)]
pub struct StockJustWithSymbolAndName {
    pub stock_symbol: String,
    pub name: String,
}
