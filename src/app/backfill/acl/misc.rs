use crate::infra::crawler::share::QfiiDto;
use rust_decimal::Decimal;

/// 外資持股狀態更新命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateQfiiCommand {
    /// 證券代號
    pub symbol: String,
    /// 外資持有股數
    pub shares_held: i64,
    /// 外資持股比率
    pub share_holding_percentage: Decimal,
    /// 已發行股數
    pub issued_share: i64,
}

/// 儲存個股權重命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveStockWeightCommand {
    /// 股票代號
    pub symbol: String,
    /// 權重百分比
    pub weight: Decimal,
}

/// 外資持股爬蟲資料防腐層轉譯器。
pub struct QfiiAclMapper;

impl QfiiAclMapper {
    /// 將原始的外資持股資料轉譯成系統內部的 `UpdateQfiiCommand`。
    pub fn from_qfii(dto: &QfiiDto) -> UpdateQfiiCommand {
        UpdateQfiiCommand {
            symbol: dto.stock_symbol.clone(),
            shares_held: dto.shares_held,
            share_holding_percentage: dto.share_holding_percentage,
            issued_share: dto.issued_share,
        }
    }
}

/// 個股權重爬蟲資料防腐層轉譯器。
pub struct StockWeightAclMapper;

impl StockWeightAclMapper {
    /// 將爬蟲取得的 `StockWeight` 轉譯為 `SaveStockWeightCommand`。
    pub fn from_dto(
        dto: &crate::infra::crawler::taifex::stock_weight::StockWeight,
    ) -> SaveStockWeightCommand {
        SaveStockWeightCommand {
            symbol: dto.stock_symbol.clone(),
            weight: dto.weight,
        }
    }
}

#[cfg(test)]
mod tests;
