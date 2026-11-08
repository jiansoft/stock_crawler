use rust_decimal::Decimal;

/// 更新每股淨值命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNetAssetValueCommand {
    /// 股票代號
    pub symbol: String,
    /// 每股淨值
    pub net_asset_value_per_share: Decimal,
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

/// 財務報表爬蟲資料防腐層轉譯器。
pub struct FinancialStatementAclMapper;

impl FinancialStatementAclMapper {
    /// 將 Wespai 爬蟲取得的 Profit DTO 轉譯為領域實體 `FinancialStatement`。
    pub fn from_wespai(
        dto: crate::infra::crawler::wespai::profit::Profit,
    ) -> crate::domain::financial::entity::FinancialStatement {
        use chrono::Local;
        crate::domain::financial::entity::FinancialStatement {
            serial: 0,
            security_code: dto.security_code,
            year: dto.year as i64,
            quarter: dto.quarter,
            gross_profit: dto.gross_profit,
            operating_profit_margin: dto.operating_profit_margin,
            pre_tax_income: dto.pre_tax_income,
            net_income: dto.net_income,
            net_asset_value_per_share: dto.net_asset_value_per_share,
            sales_per_share: dto.sales_per_share,
            earnings_per_share: dto.earnings_per_share,
            profit_before_tax: dto.profit_before_tax,
            return_on_equity: dto.return_on_equity,
            return_on_assets: dto.return_on_assets,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    /// 將 Yahoo 爬蟲取得的 Profile DTO 轉譯為領域實體 `FinancialStatement`。
    pub fn from_yahoo_profile(
        dto: crate::infra::crawler::yahoo::profile::Profile,
    ) -> crate::domain::financial::entity::FinancialStatement {
        use chrono::Local;
        crate::domain::financial::entity::FinancialStatement {
            serial: 0,
            security_code: dto.stock_symbol,
            year: dto.year as i64,
            quarter: dto.quarter,
            gross_profit: dto.gross_profit,
            operating_profit_margin: dto.operating_profit_margin,
            pre_tax_income: dto.pre_tax_income,
            net_income: dto.net_income,
            net_asset_value_per_share: dto.net_asset_value_per_share,
            sales_per_share: dto.sales_per_share,
            earnings_per_share: dto.earnings_per_share,
            profit_before_tax: dto.profit_before_tax,
            return_on_equity: dto.return_on_equity,
            return_on_assets: dto.return_on_assets,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    /// 將 TWSE/TPEX 爬蟲取得的 Eps DTO 轉譯為領域實體 `FinancialStatement`。
    pub fn from_eps(
        dto: crate::infra::crawler::twse::eps::Eps,
    ) -> crate::domain::financial::entity::FinancialStatement {
        use chrono::Local;
        crate::domain::financial::entity::FinancialStatement {
            serial: 0,
            security_code: dto.stock_symbol,
            year: dto.year as i64,
            quarter: dto.quarter.to_string(),
            gross_profit: Default::default(),
            operating_profit_margin: Default::default(),
            pre_tax_income: Default::default(),
            net_income: Default::default(),
            net_asset_value_per_share: Default::default(),
            sales_per_share: Default::default(),
            earnings_per_share: dto.earnings_per_share,
            profit_before_tax: Default::default(),
            return_on_equity: Default::default(),
            return_on_assets: Default::default(),
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    /// 將爬蟲取得的 `AnnualProfit` DTO 轉譯為領域實體 `FinancialStatement`。
    pub fn from_annual_profit(
        dto: crate::infra::crawler::share::AnnualProfit,
    ) -> crate::domain::financial::entity::FinancialStatement {
        use chrono::Local;
        crate::domain::financial::entity::FinancialStatement {
            serial: 0,
            security_code: dto.stock_symbol,
            year: dto.year as i64,
            quarter: String::new(),
            gross_profit: Default::default(),
            operating_profit_margin: Default::default(),
            pre_tax_income: Default::default(),
            net_income: Default::default(),
            net_asset_value_per_share: Default::default(),
            sales_per_share: dto.sales_per_share,
            earnings_per_share: dto.earnings_per_share,
            profit_before_tax: dto.profit_before_tax,
            return_on_equity: Default::default(),
            return_on_assets: Default::default(),
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }
}

#[cfg(test)]
mod tests;
