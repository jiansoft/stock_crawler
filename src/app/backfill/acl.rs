//! # 防腐層 (Anti-Corruption Layer)
//!
//! 用於隔離外部爬蟲資料結構（Crawler DTO）與應用層/領域層之業務邏輯命令或實體。

use crate::infra::crawler::share::{DailyQuoteDto, EtfInfo, RevenueDto};
use crate::infra::crawler::twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber;
use crate::infra::crawler::twse::suspend_listing::SuspendListing;
use crate::infra::database::table::stock::extension::qualified_foreign_institutional_investor::QualifiedForeignInstitutionalInvestor;
use chrono::{Datelike, NaiveDate};
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

/// 更新單月營收命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRevenueCommand {
    /// 股票代號
    pub symbol: String,
    /// 當月營收
    pub monthly: Decimal,
    /// 上月營收
    pub last_month: Decimal,
    /// 去年當月營收
    pub last_year_this_month: Decimal,
    /// 當月累計營收
    pub monthly_accumulated: Decimal,
    /// 去年累計營收
    pub last_year_monthly_accumulated: Decimal,
    /// 上月比較增減(%)
    pub compared_with_last_month: Decimal,
    /// 去年同月增減(%)
    pub compared_with_last_year_same_month: Decimal,
    /// 前期比較增減(%)
    pub accumulated_compared_with_last_year: Decimal,
    /// 營收月份 (YYYYMM 格式整數)
    pub date: i64,
}

/// 儲存每日收盤報價命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDailyQuoteCommand {
    /// 股票代號
    pub symbol: String,
    /// 交易日期
    pub date: NaiveDate,
    /// 開盤價
    pub opening_price: Decimal,
    /// 最高價
    pub highest_price: Decimal,
    /// 最低價
    pub lowest_price: Decimal,
    /// 收盤價
    pub closing_price: Decimal,
    /// 漲跌價差
    pub change: Decimal,
    /// 漲跌幅（百分比）
    pub change_range: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    /// 成交金額
    pub trade_value: Decimal,
    /// 成交筆數
    pub transaction: Decimal,
    /// 本益比
    pub price_earning_ratio: Decimal,
    /// 股價淨值比
    pub price_to_book_ratio: Decimal,
    /// 最後揭示買價
    pub last_best_bid_price: Decimal,
    /// 最後揭示買量
    pub last_best_bid_volume: Decimal,
    /// 最後揭示賣價
    pub last_best_ask_price: Decimal,
    /// 最後揭示賣量
    pub last_best_ask_volume: Decimal,
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

/// ETF 爬蟲資料防腐層轉譯器。
pub struct EtfAclMapper;

impl EtfAclMapper {
    /// 將原始 ETF DTO 轉譯成 `RegisterStockCommand`。
    pub fn to_registration_command(dto: &EtfInfo) -> RegisterStockCommand {
        RegisterStockCommand {
            symbol: dto.stock_symbol.clone(),
            name: dto.name.clone(),
            market_id: dto.exchange_market.stock_exchange_market_id,
            industry_id: dto.industry_id,
        }
    }
}

/// 營收爬蟲資料防腐層轉譯器。
pub struct RevenueAclMapper;

impl RevenueAclMapper {
    /// 將原始爬蟲 DTO 轉譯成 `UpdateRevenueCommand`。
    pub fn to_update_command(dto: &RevenueDto) -> UpdateRevenueCommand {
        UpdateRevenueCommand {
            symbol: dto.stock_symbol.clone(),
            monthly: dto.monthly,
            last_month: dto.last_month,
            last_year_this_month: dto.last_year_this_month,
            monthly_accumulated: dto.monthly_accumulated,
            last_year_monthly_accumulated: dto.last_year_monthly_accumulated,
            compared_with_last_month: dto.compared_with_last_month,
            compared_with_last_year_same_month: dto.compared_with_last_year_same_month,
            accumulated_compared_with_last_year: dto.accumulated_compared_with_last_year,
            date: dto.date,
        }
    }

    /// 將 `UpdateRevenueCommand` 轉譯為資料庫 Table 模型 `Revenue`。
    pub fn to_database_entity(
        cmd: &UpdateRevenueCommand,
    ) -> crate::infra::database::table::financial::revenue::Revenue {
        use chrono::Local;
        let mut r = crate::infra::database::table::financial::revenue::Revenue::new();
        r.stock_symbol = cmd.symbol.clone();
        r.monthly = cmd.monthly;
        r.last_month = cmd.last_month;
        r.last_year_this_month = cmd.last_year_this_month;
        r.monthly_accumulated = cmd.monthly_accumulated;
        r.last_year_monthly_accumulated = cmd.last_year_monthly_accumulated;
        r.compared_with_last_month = cmd.compared_with_last_month;
        r.compared_with_last_year_same_month = cmd.compared_with_last_year_same_month;
        r.accumulated_compared_with_last_year = cmd.accumulated_compared_with_last_year;
        r.date = cmd.date;
        r.create_time = Local::now();
        r
    }
}

/// 每日收盤報價爬蟲資料防腐層轉譯器。
pub struct QuoteAclMapper;

impl QuoteAclMapper {
    /// 將爬蟲 DTO 轉譯成系統內部的 `SaveDailyQuoteCommand`。
    pub fn to_save_command(dto: &DailyQuoteDto) -> SaveDailyQuoteCommand {
        SaveDailyQuoteCommand {
            symbol: dto.symbol.clone(),
            date: dto.date,
            opening_price: dto.opening_price,
            highest_price: dto.highest_price,
            lowest_price: dto.lowest_price,
            closing_price: dto.closing_price,
            change: dto.change,
            change_range: dto.change_range,
            trading_volume: dto.trading_volume,
            trade_value: dto.trade_value,
            transaction: dto.transaction,
            price_earning_ratio: dto.price_earning_ratio,
            price_to_book_ratio: dto.price_to_book_ratio,
            last_best_bid_price: dto.last_best_bid_price,
            last_best_bid_volume: dto.last_best_bid_volume,
            last_best_ask_price: dto.last_best_ask_price,
            last_best_ask_volume: dto.last_best_ask_volume,
        }
    }

    /// 將 `SaveDailyQuoteCommand` 轉譯為資料庫 Table 模型 `DailyQuote`。
    pub fn to_database_entity(
        cmd: &SaveDailyQuoteCommand,
    ) -> crate::infra::database::table::daily_quote::DailyQuote {
        use chrono::{Local, TimeZone};
        let mut dq =
            crate::infra::database::table::daily_quote::DailyQuote::new(cmd.symbol.clone());

        dq.date = cmd.date;
        dq.year = cmd.date.year();
        dq.month = cmd.date.month() as i32;
        dq.day = cmd.date.day() as i32;
        dq.opening_price = cmd.opening_price;
        dq.highest_price = cmd.highest_price;
        dq.lowest_price = cmd.lowest_price;
        dq.closing_price = cmd.closing_price;
        dq.change = cmd.change;
        dq.change_range = cmd.change_range;
        dq.trading_volume = cmd.trading_volume;
        dq.trade_value = cmd.trade_value;
        dq.transaction = cmd.transaction;
        dq.price_earning_ratio = cmd.price_earning_ratio;
        dq.price_to_book_ratio = cmd.price_to_book_ratio;
        dq.last_best_bid_price = cmd.last_best_bid_price;
        dq.last_best_bid_volume = cmd.last_best_bid_volume;
        dq.last_best_ask_price = cmd.last_best_ask_price;
        dq.last_best_ask_volume = cmd.last_best_ask_volume;

        // 台北時區 (UTC+8)
        let timezone = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        let record_time = cmd
            .date
            .and_hms_opt(15, 0, 0)
            .and_then(|naive| timezone.from_local_datetime(&naive).single())
            .unwrap_or_else(|| Local::now().with_timezone(&timezone));

        dq.record_time = record_time.with_timezone(&Local);
        dq.create_time = Local::now();

        dq
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

    #[test]
    fn test_etf_to_registration_command() {
        let etf = EtfInfo {
            stock_symbol: "0050".to_string(),
            name: "元大台灣50".to_string(),
            listing_date: "2003/06/30".to_string(),
            industry: "ETF".to_string(),
            exchange_market: StockExchangeMarket::new(2, 1),
            industry_id: 9001,
        };

        let cmd = EtfAclMapper::to_registration_command(&etf);
        assert_eq!(cmd.symbol, "0050");
        assert_eq!(cmd.name, "元大台灣50");
        assert_eq!(cmd.market_id, 2);
        assert_eq!(cmd.industry_id, 9001);
    }

    #[test]
    fn test_revenue_acl_mapping() {
        let dto = RevenueDto {
            stock_symbol: "2330".to_string(),
            monthly: dec!(1000.0),
            last_month: dec!(900.0),
            last_year_this_month: dec!(950.0),
            monthly_accumulated: dec!(5000.0),
            last_year_monthly_accumulated: dec!(4500.0),
            compared_with_last_month: dec!(11.1),
            compared_with_last_year_same_month: dec!(5.3),
            accumulated_compared_with_last_year: dec!(11.1),
            date: 202605,
        };

        let cmd = RevenueAclMapper::to_update_command(&dto);
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.monthly, dec!(1000.0));
        assert_eq!(cmd.date, 202605);

        let entity = RevenueAclMapper::to_database_entity(&cmd);
        assert_eq!(entity.stock_symbol, "2330");
        assert_eq!(entity.monthly, dec!(1000.0));
        assert_eq!(entity.date, 202605);
    }

    #[test]
    fn test_quote_acl_mapping() {
        let dto = DailyQuoteDto {
            symbol: "2330".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
            opening_price: dec!(900.0),
            highest_price: dec!(910.0),
            lowest_price: dec!(895.0),
            closing_price: dec!(905.0),
            change: dec!(5.0),
            change_range: dec!(0.55),
            trading_volume: dec!(10000),
            trade_value: dec!(9000000),
            transaction: dec!(500),
            price_earning_ratio: dec!(20.5),
            price_to_book_ratio: dec!(5.2),
            last_best_bid_price: dec!(904.0),
            last_best_bid_volume: dec!(10),
            last_best_ask_price: dec!(905.0),
            last_best_ask_volume: dec!(20),
        };

        let cmd = QuoteAclMapper::to_save_command(&dto);
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.closing_price, dec!(905.0));

        let entity = QuoteAclMapper::to_database_entity(&cmd);
        assert_eq!(entity.stock_symbol, "2330");
        assert_eq!(entity.closing_price, dec!(905.0));
        assert_eq!(entity.date, NaiveDate::from_ymd_opt(2026, 6, 5).unwrap());
    }
}
