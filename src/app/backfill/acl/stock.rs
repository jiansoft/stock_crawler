use crate::infra::cache::SHARE;
use crate::infra::crawler::share::EtfInfo;
use crate::infra::crawler::twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber;
use crate::infra::crawler::twse::suspend_listing::SuspendListing;

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

/// ISIN 爬蟲資料防腐層轉譯器。
pub struct IsinAclMapper;

impl IsinAclMapper {
    /// 將爬蟲取得的 ISIN 原始 DTO 轉譯成系統內部的 `RegisterStockCommand`。
    ///
    /// # 安全防護與過濾
    /// - 若 `industry_id` 為 `0` (代表未分類、非法或解析錯誤的產業資料)，此函式會回傳 `None` 進行過濾阻擋。
    pub fn from_isin(
        dto: &InternationalSecuritiesIdentificationNumber,
    ) -> Option<RegisterStockCommand> {
        let industry_id = SHARE.get_industry_id(&dto.industry).unwrap_or(0);
        // 1. 過濾非法、未分類產業資料 (industry_id == 0) 或 ETF (9001) / ETN (9002)
        if industry_id == 0 || industry_id == 9001 || industry_id == 9002 {
            return None;
        }

        let market_id = SHARE
            .get_exchange_market(dto.market.serial())
            .unwrap()
            .stock_exchange_market_id;

        // 2. 轉換為內部的 RegisterStockCommand
        Some(RegisterStockCommand {
            symbol: dto.stock_symbol.clone(),
            name: dto.name.clone(),
            market_id,
            industry_id,
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
    pub fn from_suspend_listing(dto: &SuspendListing) -> Option<DelistedCompanyCommand> {
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

/// ETF 爬蟲資料防腐層轉譯器。
pub struct EtfAclMapper;

impl EtfAclMapper {
    /// 將原始 ETF DTO 轉譯成 `RegisterStockCommand`。
    pub fn from_etf(dto: &EtfInfo) -> RegisterStockCommand {
        let market_id = SHARE
            .get_exchange_market(dto.market.serial())
            .unwrap()
            .stock_exchange_market_id;
        RegisterStockCommand {
            symbol: dto.stock_symbol.clone(),
            name: dto.name.clone(),
            market_id,
            industry_id: dto.industry_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_registration_command_success() {
        let isin = InternationalSecuritiesIdentificationNumber {
            stock_symbol: "2330".to_string(),
            name: "台積電".to_string(),
            isin_code: "TW0002330008".to_string(),
            listing_date: "1994/09/05".to_string(),
            industry: "半導體業".to_string(),
            cfi_code: "ESVUFR".to_string(),
            market: crate::core::declare::StockExchangeMarket::Listed,
        };

        let cmd = IsinAclMapper::from_isin(&isin);
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
            market: crate::core::declare::StockExchangeMarket::Listed,
        };

        let cmd = IsinAclMapper::from_isin(&isin);
        assert!(cmd.is_none());
    }

    #[test]
    fn test_to_delisted_command_success() {
        let dto = SuspendListing {
            delisting_date: "1120520".to_string(),
            name: "測試下市".to_string(),
            stock_symbol: "9999".to_string(),
        };

        let cmd = DelistedCompanyAclMapper::from_suspend_listing(&dto);
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

        let cmd = DelistedCompanyAclMapper::from_suspend_listing(&dto);
        assert!(cmd.is_none(), "應過濾掉民國 110 年之前的下市資料");
    }

    #[test]
    fn test_to_delisted_command_invalid_date() {
        let dto = SuspendListing {
            delisting_date: "12".to_string(),
            name: "無效日期".to_string(),
            stock_symbol: "9999".to_string(),
        };

        let cmd = DelistedCompanyAclMapper::from_suspend_listing(&dto);
        assert!(cmd.is_none(), "日期太短應過濾");
    }

    #[test]
    fn test_etf_to_registration_command() {
        let etf = EtfInfo {
            stock_symbol: "0050".to_string(),
            name: "元大台灣50".to_string(),
            listing_date: "2003/06/30".to_string(),
            industry: "ETF".to_string(),
            market: crate::core::declare::StockExchangeMarket::Listed,
            industry_id: 9001,
        };

        let cmd = EtfAclMapper::from_etf(&etf);
        assert_eq!(cmd.symbol, "0050");
        assert_eq!(cmd.name, "元大台灣50");
        assert_eq!(cmd.market_id, 2);
        assert_eq!(cmd.industry_id, 9001);
    }
}
