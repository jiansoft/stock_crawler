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

/// 儲存大盤加權指數命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveIndexCommand {
    /// 日期
    pub date: NaiveDate,
    /// 收盤指數
    pub index: Decimal,
    /// 漲跌點數
    pub change: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    /// 成交金額
    pub trade_value: Decimal,
    /// 成交筆數
    pub transaction: Decimal,
}

/// 儲存個股權重命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveStockWeightCommand {
    /// 股票代號
    pub symbol: String,
    /// 權重百分比
    pub weight: Decimal,
}

/// 更新每股淨值命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNetAssetValueCommand {
    /// 股票代號
    pub symbol: String,
    /// 每股淨值
    pub net_asset_value_per_share: Decimal,
}

/// 更新股息分配率命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePayoutRatioCommand {
    /// 股利資料序號
    pub serial: i64,
    /// 現金配發率
    pub payout_ratio_cash: Decimal,
    /// 股票配發率
    pub payout_ratio_stock: Decimal,
    /// 合計配發率
    pub payout_ratio: Decimal,
}

/// 儲存/更新股息明細命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDividendCommand {
    /// 股票代碼
    pub security_code: String,
    /// 發放年度
    pub year: i32,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 季度/半年資訊
    pub quarter: String,
    /// 現金股利
    pub cash_dividend: Decimal,
    /// 股票股利
    pub stock_dividend: Decimal,
    /// 股利合計
    pub sum: Decimal,
    /// 除息日
    pub ex_dividend_date1: String,
    /// 除權日
    pub ex_dividend_date2: String,
    /// 現金股利發放日
    pub payable_date1: String,
    /// 股票股利發放日
    pub payable_date2: String,
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

/// 外資持股爬蟲資料防腐層轉譯器。
pub struct QfiiAclMapper;

impl QfiiAclMapper {
    /// 將原始的外資持股資料轉譯成系統內部的 `UpdateQfiiCommand`。
    pub fn from_qfii(dto: &QualifiedForeignInstitutionalInvestor) -> UpdateQfiiCommand {
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
    pub fn from_etf(dto: &EtfInfo) -> RegisterStockCommand {
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
    pub fn from_dto(dto: &RevenueDto) -> UpdateRevenueCommand {
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
    pub fn from_command(
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
    pub fn from_dto(dto: &DailyQuoteDto) -> SaveDailyQuoteCommand {
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
    pub fn from_command(
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

/// 大盤加權指數爬蟲資料防腐層轉譯器。
pub struct IndexAclMapper;

impl IndexAclMapper {
    /// 將爬蟲取得的大盤原始字串資料列轉譯為 `SaveIndexCommand`。
    pub fn from_strings(item: &[String]) -> Option<SaveIndexCommand> {
        if item.len() != 6 {
            return None;
        }

        let split_date: Vec<&str> = item[0].split('/').collect();
        if split_date.len() != 3 {
            return None;
        }

        let year = split_date[0].parse::<i32>().ok()?;
        let gregorian_year = crate::core::util::datetime::roc_year_to_gregorian_year(year);
        let date_str = format!("{}-{}-{}", gregorian_year, split_date[1], split_date[2]);
        let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").ok()?;

        let trading_volume = crate::core::util::text::parse_decimal(&item[1], None).ok()?;
        let trade_value = crate::core::util::text::parse_decimal(&item[2], None).ok()?;
        let transaction = crate::core::util::text::parse_decimal(&item[3], None).ok()?;
        let index = crate::core::util::text::parse_decimal(&item[4], None).ok()?;
        let change = crate::core::util::text::parse_decimal(&item[5], None).ok()?;

        Some(SaveIndexCommand {
            date,
            index,
            change,
            trading_volume,
            trade_value,
            transaction,
        })
    }

    /// 將 `SaveIndexCommand` 轉譯為資料庫 Table 模型 `Index`。
    pub fn from_command(cmd: &SaveIndexCommand) -> crate::infra::database::table::index::Index {
        use chrono::Local;
        let mut entity = crate::infra::database::table::index::Index::new();
        entity.category = "TAIEX".to_string();
        entity.date = cmd.date;
        entity.index = cmd.index;
        entity.change = cmd.change;
        entity.trading_volume = cmd.trading_volume;
        entity.trade_value = cmd.trade_value;
        entity.transaction = cmd.transaction;
        entity.create_time = Local::now();
        entity.update_time = Local::now();
        entity
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

    /// 將 `SaveStockWeightCommand` 轉譯為資料庫 Table 模型 `SymbolAndWeight`。
    pub fn from_command(
        cmd: &SaveStockWeightCommand,
    ) -> crate::infra::database::table::stock::extension::weight::SymbolAndWeight {
        crate::infra::database::table::stock::extension::weight::SymbolAndWeight::new(
            cmd.symbol.clone(),
            cmd.weight,
        )
    }
}

/// 每股淨值爬蟲資料防腐層轉譯器。
pub struct NetAssetValueAclMapper;

impl NetAssetValueAclMapper {
    /// 將興櫃 DTO 轉譯為 `UpdateNetAssetValueCommand`。
    pub fn from_emerging(
        dto: &crate::infra::crawler::tpex::net_asset_value_per_share::Emerging,
    ) -> UpdateNetAssetValueCommand {
        UpdateNetAssetValueCommand {
            symbol: dto.stock_symbol.clone(),
            net_asset_value_per_share: dto.net_asset_value_per_share,
        }
    }

    /// 將 Yahoo DTO 轉譯為 `UpdateNetAssetValueCommand`。
    pub fn from_yahoo_profile(
        symbol: String,
        dto: &crate::infra::crawler::yahoo::profile::Profile,
    ) -> UpdateNetAssetValueCommand {
        UpdateNetAssetValueCommand {
            symbol,
            net_asset_value_per_share: dto.net_asset_value_per_share,
        }
    }
}

/// 股息分配率爬蟲資料防腐層轉譯器。
pub struct DividendAclMapper;

impl DividendAclMapper {
    /// 將 Goodinfo 股利 DTO 轉譯為 `UpdatePayoutRatioCommand`。
    pub fn from_dto(
        serial: i64,
        dto: &crate::infra::crawler::goodinfo::dividend::GoodInfoDividend,
    ) -> UpdatePayoutRatioCommand {
        UpdatePayoutRatioCommand {
            serial,
            payout_ratio_cash: dto.payout_ratio_cash,
            payout_ratio_stock: dto.payout_ratio_stock,
            payout_ratio: dto.payout_ratio,
        }
    }

    /// 將 `UpdatePayoutRatioCommand` 套用至 `PayoutRatioInfo`。
    pub fn update_payout_ratio_entity(
        pri: &crate::infra::database::table::dividend::extension::payout_ratio_info::PayoutRatioInfo,
        cmd: &UpdatePayoutRatioCommand,
    ) -> crate::infra::database::table::dividend::extension::payout_ratio_info::PayoutRatioInfo
    {
        crate::infra::database::table::dividend::extension::payout_ratio_info::PayoutRatioInfo {
            serial: cmd.serial,
            year: pri.year,
            quarter: pri.quarter.clone(),
            security_code: pri.security_code.clone(),
            payout_ratio_cash: cmd.payout_ratio_cash,
            payout_ratio_stock: cmd.payout_ratio_stock,
            payout_ratio: cmd.payout_ratio,
        }
    }
}

/// Yahoo 股利明細爬蟲資料防腐層轉譯器。
pub struct YahooDividendAclMapper;

impl YahooDividendAclMapper {
    /// 將 Yahoo 股利明細 DTO 轉譯為 `SaveDividendCommand`。
    pub fn from_dto(
        stock_symbol: &str,
        dto: &crate::infra::crawler::yahoo::dividend::YahooDividendDetail,
    ) -> SaveDividendCommand {
        SaveDividendCommand {
            security_code: stock_symbol.to_string(),
            year: dto.year,
            year_of_dividend: dto.year_of_dividend,
            quarter: dto.quarter.clone(),
            cash_dividend: dto.cash_dividend,
            stock_dividend: dto.stock_dividend,
            sum: dto.cash_dividend + dto.stock_dividend,
            ex_dividend_date1: dto.ex_dividend_date1.clone(),
            ex_dividend_date2: dto.ex_dividend_date2.clone(),
            payable_date1: dto.payable_date1.clone(),
            payable_date2: dto.payable_date2.clone(),
        }
    }

    /// 將 `SaveDividendCommand` 轉譯為資料庫 Table 模型 `Dividend`。
    pub fn from_command(
        cmd: &SaveDividendCommand,
    ) -> crate::infra::database::table::dividend::Dividend {
        use chrono::Local;
        let mut e = crate::infra::database::table::dividend::Dividend::new();
        e.security_code = cmd.security_code.clone();
        e.year = cmd.year;
        e.year_of_dividend = cmd.year_of_dividend;
        e.quarter = cmd.quarter.clone();
        e.cash_dividend = cmd.cash_dividend;
        e.stock_dividend = cmd.stock_dividend;
        e.sum = cmd.sum;
        e.ex_dividend_date1 = cmd.ex_dividend_date1.clone();
        e.ex_dividend_date2 = cmd.ex_dividend_date2.clone();
        e.payable_date1 = cmd.payable_date1.clone();
        e.payable_date2 = cmd.payable_date2.clone();
        e.created_time = Local::now();
        e.updated_time = Local::now();
        e
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
            exchange_market: StockExchangeMarket::new(2, 1),
            industry_id: 0,
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
    fn test_qfii_to_update_command() {
        let dto = QualifiedForeignInstitutionalInvestor::new(
            "2330".to_string(),
            100000,
            50000,
            dec!(75.5),
        );

        let cmd = QfiiAclMapper::from_qfii(&dto);
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

        let cmd = EtfAclMapper::from_etf(&etf);
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

        let cmd = RevenueAclMapper::from_dto(&dto);
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.monthly, dec!(1000.0));
        assert_eq!(cmd.date, 202605);

        let entity = RevenueAclMapper::from_command(&cmd);
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

        let cmd = QuoteAclMapper::from_dto(&dto);
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.closing_price, dec!(905.0));

        let entity = QuoteAclMapper::from_command(&cmd);
        assert_eq!(entity.stock_symbol, "2330");
        assert_eq!(entity.closing_price, dec!(905.0));
        assert_eq!(entity.date, NaiveDate::from_ymd_opt(2026, 6, 5).unwrap());
    }

    #[test]
    fn test_index_acl_mapping() {
        let item = vec![
            "112/05/20".to_string(), // date
            "1,234,567".to_string(), // trading_volume
            "4,567,890".to_string(), // trade_value
            "100,000".to_string(),   // transaction
            "16,000.50".to_string(), // index
            "150.25".to_string(),    // change
        ];

        let cmd = IndexAclMapper::from_strings(&item);
        assert!(cmd.is_some());
        let cmd = cmd.unwrap();
        assert_eq!(cmd.date, NaiveDate::from_ymd_opt(2023, 5, 20).unwrap());
        assert_eq!(cmd.index, dec!(16000.50));
        assert_eq!(cmd.change, dec!(150.25));

        let entity = IndexAclMapper::from_command(&cmd);
        assert_eq!(entity.category, "TAIEX");
        assert_eq!(entity.index, dec!(16000.50));
        assert_eq!(entity.change, dec!(150.25));
    }

    #[test]
    fn test_stock_weight_acl_mapping() {
        let dto = crate::infra::crawler::taifex::stock_weight::StockWeight {
            rank: 1,
            stock_symbol: "2330".to_string(),
            weight: dec!(28.5),
        };

        let cmd = StockWeightAclMapper::from_dto(&dto);
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.weight, dec!(28.5));

        let entity = StockWeightAclMapper::from_command(&cmd);
        assert_eq!(entity.stock_symbol, "2330");
        assert_eq!(entity.weight, dec!(28.5));
    }

    #[test]
    fn test_net_asset_value_acl_mapping() {
        let emerging = crate::infra::crawler::tpex::net_asset_value_per_share::Emerging {
            stock_symbol: "6987".to_string(),
            net_asset_value_per_share: dec!(42.19),
        };
        let cmd1 = NetAssetValueAclMapper::from_emerging(&emerging);
        assert_eq!(cmd1.symbol, "6987");
        assert_eq!(cmd1.net_asset_value_per_share, dec!(42.19));

        let profile = crate::infra::crawler::yahoo::profile::Profile {
            net_asset_value_per_share: dec!(95.12),
            ..Default::default()
        };
        let cmd2 = NetAssetValueAclMapper::from_yahoo_profile("2330".to_string(), &profile);
        assert_eq!(cmd2.symbol, "2330");
        assert_eq!(cmd2.net_asset_value_per_share, dec!(95.12));
    }

    #[test]
    fn test_dividend_acl_mapping() {
        let mut goodinfo =
            crate::infra::crawler::goodinfo::dividend::GoodInfoDividend::new("2330".to_string());
        goodinfo.payout_ratio_cash = dec!(45.5);
        goodinfo.payout_ratio_stock = dec!(0.0);
        goodinfo.payout_ratio = dec!(45.5);

        let cmd = DividendAclMapper::from_dto(123, &goodinfo);
        assert_eq!(cmd.serial, 123);
        assert_eq!(cmd.payout_ratio_cash, dec!(45.5));

        let mut pri = crate::infra::database::table::dividend::extension::payout_ratio_info::PayoutRatioInfo {
            serial: 123,
            year: 2024,
            quarter: "Q4".to_string(),
            security_code: "2330".to_string(),
            payout_ratio_cash: dec!(0.0),
            payout_ratio_stock: dec!(0.0),
            payout_ratio: dec!(0.0),
        };
        pri = DividendAclMapper::update_payout_ratio_entity(&pri, &cmd);
        assert_eq!(pri.payout_ratio_cash, dec!(45.5));
    }

    #[test]
    fn test_yahoo_dividend_acl_mapping() {
        let detail = crate::infra::crawler::yahoo::dividend::YahooDividendDetail {
            year: 2025,
            year_of_dividend: 2024,
            quarter: "Q4".to_string(),
            cash_dividend: dec!(3.5),
            stock_dividend: dec!(0.2),
            ex_dividend_date1: "2025-07-01".to_string(),
            ex_dividend_date2: "-".to_string(),
            payable_date1: "2025-08-01".to_string(),
            payable_date2: "-".to_string(),
        };

        let cmd = YahooDividendAclMapper::from_dto("2454", &detail);
        assert_eq!(cmd.security_code, "2454");
        assert_eq!(cmd.sum, dec!(3.7));

        let entity = YahooDividendAclMapper::from_command(&cmd);
        assert_eq!(entity.security_code, "2454");
        assert_eq!(entity.sum, dec!(3.7));
        assert_eq!(entity.ex_dividend_date1, "2025-07-01");
    }
}
