//! # 防腐層 (Anti-Corruption Layer)
//!
//! 用於隔離外部爬蟲資料結構（Crawler DTO）與應用層/領域層之業務邏輯命令或實體。

use crate::infra::crawler::twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber;

/// 註冊或變更證券識別資料命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterStockCommand {
    /// 證券代號
    pub symbol: String,
    /// 證券名稱
    pub name: String,
    /// 交易所市場 ID
    pub market_id: i32,
    /// 產業分類 ID
    pub industry_id: i32,
}

/// ISIN 爬蟲資料防腐層轉譯器。
pub struct IsinAclMapper;

impl IsinAclMapper {
    /// 將爬蟲取得的 ISIN 原始 DTO 轉譯成系統內部的 `RegisterStockCommand`。
    ///
    /// # 安全防護與過濾
    /// - 若 `industry_id` 為 `0` (代表未分類、非法或解析錯誤的產業資料)，此函式會回傳 `None` 進行過濾阻擋。
    pub fn to_registration_command(
        dto: &InternationalSecuritiesIdentificationNumber,
    ) -> Option<RegisterStockCommand> {
        // 1. 過濾非法或未分類產業資料 (industry_id == 0)
        if dto.industry_id == 0 {
            return None;
        }

        // 2. 轉換為內部的 RegisterStockCommand
        Some(RegisterStockCommand {
            symbol: dto.stock_symbol.clone(),
            name: dto.name.clone(),
            market_id: dto.exchange_market.stock_exchange_market_id,
            industry_id: dto.industry_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::database::table::stock_exchange_market::StockExchangeMarket;

    #[test]
    fn test_to_registration_command_success() {
        let isin = InternationalSecuritiesIdentificationNumber {
            stock_symbol: "2330".to_string(),
            name: "台積電".to_string(),
            isin_code: "TW0002330008".to_string(),
            listing_date: "1994/09/05".to_string(),
            industry: "半導體業".to_string(),
            cfi_code: "ESVUFR".to_string(),
            exchange_market: StockExchangeMarket::new(2, 1),
            industry_id: 24,
        };

        let cmd = IsinAclMapper::to_registration_command(&isin);
        assert!(cmd.is_some());
        let cmd = cmd.unwrap();
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.name, "台積電");
        assert_eq!(cmd.market_id, 2);
        assert_eq!(cmd.industry_id, 24);
    }

    #[test]
    fn test_to_registration_command_invalid_industry() {
        let isin = InternationalSecuritiesIdentificationNumber {
            stock_symbol: "0050".to_string(),
            name: "元大台灣50".to_string(),
            isin_code: "TW0000050004".to_string(),
            listing_date: "2003/06/30".to_string(),
            industry: "ETF".to_string(),
            cfi_code: "CEOGEU".to_string(),
            exchange_market: StockExchangeMarket::new(2, 1),
            industry_id: 0, // 設為無效的 0
        };

        let cmd = IsinAclMapper::to_registration_command(&isin);
        assert!(cmd.is_none());
    }
}
