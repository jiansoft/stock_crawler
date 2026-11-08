//! 財報防腐層轉換的測試。

use super::*;
use rust_decimal_macros::dec;

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
fn test_financial_statement_acl_mapping() {
    let profit_dto = crate::infra::crawler::wespai::profit::Profit {
        security_code: "2330".to_string(),
        year: 2025,
        quarter: "Q1".to_string(),
        gross_profit: dec!(52.5),
        operating_profit_margin: dec!(42.0),
        pre_tax_income: dec!(45.0),
        net_income: dec!(38.0),
        net_asset_value_per_share: dec!(95.0),
        sales_per_share: dec!(12.5),
        earnings_per_share: dec!(8.1),
        profit_before_tax: dec!(9.5),
        return_on_equity: dec!(25.0),
        return_on_assets: dec!(15.0),
    };

    let entity = FinancialStatementAclMapper::from_wespai(profit_dto);
    assert_eq!(entity.security_code, "2330");
    assert_eq!(entity.year, 2025);
    assert_eq!(entity.quarter, "Q1");
    assert_eq!(entity.gross_profit, dec!(52.5));
    assert_eq!(entity.earnings_per_share, dec!(8.1));
}

#[test]
fn test_financial_statement_from_yahoo_profile_maps_all_fields() {
    let profile = crate::infra::crawler::yahoo::profile::Profile {
        stock_symbol: "2454".to_string(),
        year: 2024,
        quarter: "Q3".to_string(),
        gross_profit: dec!(60.1),
        operating_profit_margin: dec!(38.5),
        pre_tax_income: dec!(41.0),
        net_income: dec!(35.2),
        net_asset_value_per_share: dec!(88.0),
        sales_per_share: dec!(15.3),
        earnings_per_share: dec!(9.2),
        profit_before_tax: dec!(10.1),
        return_on_equity: dec!(22.0),
        return_on_assets: dec!(12.0),
    };

    let entity = FinancialStatementAclMapper::from_yahoo_profile(profile);
    assert_eq!(entity.security_code, "2454");
    assert_eq!(entity.year, 2024);
    assert_eq!(entity.quarter, "Q3");
    assert_eq!(entity.gross_profit, dec!(60.1));
    assert_eq!(entity.earnings_per_share, dec!(9.2));
    assert_eq!(entity.return_on_equity, dec!(22.0));
}

#[test]
fn test_financial_statement_from_eps_maps_symbol_year_quarter_eps() {
    use crate::core::declare::Quarter;
    use crate::infra::crawler::twse::eps::Eps;

    let eps = Eps::new("6770".to_string(), 2023, Quarter::Q2, dec!(5.78));
    let entity = FinancialStatementAclMapper::from_eps(eps);

    assert_eq!(entity.security_code, "6770");
    assert_eq!(entity.year, 2023);
    assert_eq!(entity.quarter, "Q2");
    assert_eq!(entity.earnings_per_share, dec!(5.78));
    assert_eq!(entity.gross_profit, dec!(0));
    assert_eq!(entity.sales_per_share, dec!(0));
}

#[test]
fn test_financial_statement_from_annual_profit_maps_key_fields() {
    use crate::infra::crawler::share::AnnualProfit;

    let ap = AnnualProfit {
        stock_symbol: "2412".to_string(),
        year: 2022,
        sales_per_share: dec!(20.5),
        earnings_per_share: dec!(11.3),
        profit_before_tax: dec!(13.0),
    };

    let entity = FinancialStatementAclMapper::from_annual_profit(ap);
    assert_eq!(entity.security_code, "2412");
    assert_eq!(entity.year, 2022);
    assert_eq!(entity.quarter, "");
    assert_eq!(entity.sales_per_share, dec!(20.5));
    assert_eq!(entity.earnings_per_share, dec!(11.3));
    assert_eq!(entity.profit_before_tax, dec!(13.0));
    assert_eq!(entity.gross_profit, dec!(0));
}
