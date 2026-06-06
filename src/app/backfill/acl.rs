//! # 防腐層 (Anti-Corruption Layer)
//!
//! 用於隔離外部爬蟲資料結構（Crawler DTO）與應用層/領域層之業務邏輯命令或實體。

use crate::infra::crawler::twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber;
use crate::infra::crawler::twse::suspend_listing::SuspendListing;
use crate::infra::database::table::stock::extension::qualified_foreign_institutional_investor::QualifiedForeignInstitutionalInvestor;
use rust_decimal::Decimal;

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

/// 終止上市 (下市) 處理命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelistedCompanyCommand {
    /// 證券代號
    pub symbol: String,
}

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

/// 下市公司爬蟲資料防腐層轉譯器。
pub struct DelistedCompanyAclMapper;

impl DelistedCompanyAclMapper {
    /// 將下市公司原始 DTO 轉譯成 `DelistedCompanyCommand`。
    ///
    /// # 資料清洗與過濾
    /// - 若下市日期格式無效（長度小於 3），回傳 `None`。
    /// - 過濾掉民國 110 年之前的下市資料，以縮小回補與更新範圍。
    pub fn to_delisted_command(dto: &SuspendListing) -> Option<DelistedCompanyCommand> {
        if dto.delisting_date.len() < 3 {
            return None;
        }

        // 擷取民國年前 3 碼並解析為整數
        let year = dto.delisting_date[..3].parse::<i32>().ok()?;
        if year < 110 {
            return None;
        }

        Some(DelistedCompanyCommand {
            symbol: dto.stock_symbol.clone(),
        })
    }
}

/// 外資持股爬蟲資料防腐層轉譯器。
pub struct QfiiAclMapper;

impl QfiiAclMapper {
    /// 將原始的外資持股資料轉譯成系統內部的 `UpdateQfiiCommand`。
    pub fn to_update_command(dto: &QualifiedForeignInstitutionalInvestor) -> UpdateQfiiCommand {
        UpdateQfiiCommand {
            symbol: dto.stock_symbol.clone(),
            shares_held: dto.qfii_shares_held,
            share_holding_percentage: dto.qfii_share_holding_percentage,
            issued_share: dto.issued_share,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::database::table::stock_exchange_market::StockExchangeMarket;
    use rust_decimal_macros::dec;

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
            industry_id: 0,
        };

        let cmd = IsinAclMapper::to_registration_command(&isin);
        assert!(cmd.is_none());
    }

    #[test]
    fn test_to_delisted_command_success() {
        let dto = SuspendListing {
            delisting_date: "1120520".to_string(),
            name: "測試下市".to_string(),
            stock_symbol: "9999".to_string(),
        };

        let cmd = DelistedCompanyAclMapper::to_delisted_command(&dto);
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().symbol, "9999");
    }

    #[test]
    fn test_to_delisted_command_too_old() {
        let dto = SuspendListing {
            delisting_date: "1091231".to_string(),
            name: "測試老舊下市".to_string(),
            stock_symbol: "9999".to_string(),
        };

        let cmd = DelistedCompanyAclMapper::to_delisted_command(&dto);
        assert!(cmd.is_none(), "應過濾掉民國 110 年之前的下市資料");
    }

    #[test]
    fn test_to_delisted_command_invalid_date() {
        let dto = SuspendListing {
            delisting_date: "12".to_string(),
            name: "無效日期".to_string(),
            stock_symbol: "9999".to_string(),
        };

        let cmd = DelistedCompanyAclMapper::to_delisted_command(&dto);
        assert!(cmd.is_none(), "日期太短應過濾");
    }

    #[test]
    fn test_qfii_to_update_command() {
        let dto = QualifiedForeignInstitutionalInvestor::new(
            "2330".to_string(),
            100000,
            50000,
            dec!(75.5),
        );

        let cmd = QfiiAclMapper::to_update_command(&dto);
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.shares_held, 50000);
        assert_eq!(cmd.share_holding_percentage, dec!(75.5));
        assert_eq!(cmd.issued_share, 100000);
    }
}
