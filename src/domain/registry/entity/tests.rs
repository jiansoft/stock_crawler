use super::*;
use rust_decimal_macros::dec;

#[test]
fn test_stock_symbol_is_preference() {
    assert!(StockSymbol("2881A".to_string()).is_preference());
    assert!(StockSymbol("2330B".to_string()).is_preference());
    assert!(!StockSymbol("2330".to_string()).is_preference());
    assert!(!StockSymbol("0050".to_string()).is_preference());
}

#[test]
fn test_stock_symbol_is_tdr() {
    assert!(StockSymbol("9105".to_string()).is_tdr());
    assert!(StockSymbol("9136".to_string()).is_tdr());
    assert!(!StockSymbol("2330".to_string()).is_tdr());
}

#[test]
fn test_stock_register() {
    let stock = Stock::register("2330".to_string(), "台積電".to_string(), 2, 24);

    assert_eq!(stock.symbol().0, "2330");
    assert_eq!(stock.name(), "台積電");
    assert_eq!(stock.market_id(), 2);
    assert_eq!(stock.industry_id(), 24);
    assert_eq!(stock.net_asset_value_per_share(), Decimal::ZERO);
    assert!(!stock.suspend_listing());

    let mut events = stock.clone().pull_events();
    assert_eq!(events.len(), 1);
    if let Some(DomainEvent::StockRegistered {
        symbol,
        name,
        market_id,
        industry_id,
        ..
    }) = events.pop()
    {
        assert_eq!(symbol, "2330");
        assert_eq!(name, "台積電");
        assert_eq!(market_id, 2);
        assert_eq!(industry_id, 24);
    } else {
        panic!("Expected StockRegistered event");
    }
}

#[test]
fn test_stock_reconstitute() {
    let created_time = Local::now();
    let stock = Stock::reconstitute(
        "2330".to_string(),
        "台積電".to_string(),
        true,
        dec!(95.12),
        dec!(31.5),
        dec!(25.7),
        created_time,
        2,
        24,
        25000,
        12000,
        dec!(48.5),
    );

    assert_eq!(stock.symbol().0, "2330");
    assert_eq!(stock.name(), "台積電");
    assert!(stock.suspend_listing());
    assert_eq!(stock.net_asset_value_per_share(), dec!(95.12));
    assert_eq!(stock.weight(), dec!(31.5));
    assert_eq!(stock.return_on_equity(), dec!(25.7));
    assert_eq!(stock.created_time(), created_time);
    assert_eq!(stock.market_id(), 2);
    assert_eq!(stock.industry_id(), 24);
    assert_eq!(stock.issued_share(), 25000);
    assert_eq!(stock.qfii_shares_held(), 12000);
    assert_eq!(stock.qfii_share_holding_percentage(), dec!(48.5));

    // Reconstitution must not generate any domain events
    assert!(stock.clone().pull_events().is_empty());
}

#[test]
fn test_stock_change_identity() {
    let mut stock = Stock::register("2330".to_string(), "台積電".to_string(), 2, 24);
    let _ = stock.pull_events(); // Clear registered event

    // Change with identical values should not trigger event
    stock.change_identity("台積電".to_string(), 2, 24);
    assert!(stock.pull_events().is_empty());

    // Change with different values should trigger event
    stock.change_identity("台積電新".to_string(), 3, 25);
    assert_eq!(stock.name(), "台積電新");
    assert_eq!(stock.market_id(), 3);
    assert_eq!(stock.industry_id(), 25);

    let mut events = stock.pull_events();
    assert_eq!(events.len(), 1);
    if let Some(DomainEvent::StockIdentityChanged {
        symbol,
        old_name,
        new_name,
        old_market_id,
        new_market_id,
        old_industry_id,
        new_industry_id,
        ..
    }) = events.pop()
    {
        assert_eq!(symbol, "2330");
        assert_eq!(old_name, "台積電");
        assert_eq!(new_name, "台積電新");
        assert_eq!(old_market_id, 2);
        assert_eq!(new_market_id, 3);
        assert_eq!(old_industry_id, 24);
        assert_eq!(new_industry_id, 25);
    } else {
        panic!("Expected StockIdentityChanged event");
    }
}

#[test]
fn test_stock_update_net_asset_value() {
    let mut stock = Stock::register("2330".to_string(), "台積電".to_string(), 2, 24);
    let _ = stock.pull_events(); // Clear registered event

    // Same NAV should not trigger event
    stock.update_net_asset_value(Decimal::ZERO);
    assert!(stock.pull_events().is_empty());

    // Different NAV should trigger event
    stock.update_net_asset_value(dec!(95.12));
    assert_eq!(stock.net_asset_value_per_share(), dec!(95.12));

    let mut events = stock.pull_events();
    assert_eq!(events.len(), 1);
    if let Some(DomainEvent::NetAssetValueUpdated {
        symbol,
        old_nav,
        new_nav,
        ..
    }) = events.pop()
    {
        assert_eq!(symbol, "2330");
        assert_eq!(old_nav, Decimal::ZERO);
        assert_eq!(new_nav, dec!(95.12));
    } else {
        panic!("Expected NetAssetValueUpdated event");
    }
}

#[test]
fn test_stock_other_updates() {
    let mut stock = Stock::register("2330".to_string(), "台積電".to_string(), 2, 24);

    stock.update_suspension(true);
    assert!(stock.suspend_listing());

    stock.update_qfii(12000, dec!(48.5));
    assert_eq!(stock.qfii_shares_held(), 12000);
    assert_eq!(stock.qfii_share_holding_percentage(), dec!(48.5));

    stock.update_issued_shares(25000);
    assert_eq!(stock.issued_share(), 25000);

    stock.update_weight(dec!(31.5));
    assert_eq!(stock.weight(), dec!(31.5));

    stock.update_roe(dec!(25.7));
    assert_eq!(stock.return_on_equity(), dec!(25.7));
}
