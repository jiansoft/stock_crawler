use super::*;

/// 驗證產業序號對照。
#[test]
fn test_industry_serial() {
    assert_eq!(Industry::Cement.serial(), 1);
    assert_eq!(Industry::Food.serial(), 2);
    assert_eq!(Industry::Plastic.serial(), 3);
    assert_eq!(Industry::TextileFiber.serial(), 4);
    assert_eq!(Industry::ElectricalMachinery.serial(), 5);
    assert_eq!(Industry::ElectricalCable.serial(), 6);
    assert_eq!(Industry::GlassCeramics.serial(), 8);
    assert_eq!(Industry::Paper.serial(), 9);
    assert_eq!(Industry::Steel.serial(), 10);
    assert_eq!(Industry::Rubber.serial(), 11);
    assert_eq!(Industry::Automotive.serial(), 12);
    assert_eq!(Industry::Electronic.serial(), 13);
    assert_eq!(Industry::ConstructionMaterial.serial(), 14);
    assert_eq!(Industry::Shipping.serial(), 15);
    assert_eq!(Industry::TourismCatering.serial(), 16);
    assert_eq!(Industry::FinanceInsurance.serial(), 17);
    assert_eq!(Industry::TradingDepartmentStores.serial(), 18);
    assert_eq!(Industry::Comprehensive.serial(), 19);
    assert_eq!(Industry::Other.serial(), 20);
    assert_eq!(Industry::Chemical.serial(), 21);
    assert_eq!(Industry::BiotechMedical.serial(), 22);
    assert_eq!(Industry::OilElectricGas.serial(), 23);
    assert_eq!(Industry::Semiconductor.serial(), 24);
    assert_eq!(Industry::ComputerPeripheral.serial(), 25);
    assert_eq!(Industry::Optoelectronic.serial(), 26);
    assert_eq!(Industry::CommunicationNetwork.serial(), 27);
    assert_eq!(Industry::ElectronicComponents.serial(), 28);
    assert_eq!(Industry::ElectronicPathway.serial(), 29);
    assert_eq!(Industry::InformationService.serial(), 30);
    assert_eq!(Industry::OtherElectronics.serial(), 31);
    assert_eq!(Industry::CulturalCreative.serial(), 32);
    assert_eq!(Industry::AgriculturalTechnology.serial(), 33);
    assert_eq!(Industry::ECommerce.serial(), 34);
    assert_eq!(Industry::GreenEnergyEnvironmentalProtection.serial(), 35);
    assert_eq!(Industry::DigitalCloud.serial(), 36);
    assert_eq!(Industry::SportsRecreation.serial(), 37);
    assert_eq!(Industry::HomeLife.serial(), 38);
    assert_eq!(Industry::DepositaryReceipts.serial(), 39);
    assert_eq!(Industry::ExchangeTradedFund.serial(), 9001);
    assert_eq!(Industry::ExchangeTradedNote.serial(), 9002);
    assert_eq!(Industry::BeneficialSecurity.serial(), 9003);
    assert_eq!(Industry::Uncategorized.serial(), 99);
}

/// 驗證產業名稱對照。
#[test]
fn test_industry_name() {
    assert_eq!(Industry::Cement.name(), "水泥工業");
    assert_eq!(Industry::Food.name(), "食品工業");
    assert_eq!(Industry::Plastic.name(), "塑膠工業");
    assert_eq!(Industry::TextileFiber.name(), "紡織纖維");
    assert_eq!(Industry::ElectricalMachinery.name(), "電機機械");
    assert_eq!(Industry::ElectricalCable.name(), "電器電纜");
    assert_eq!(Industry::GlassCeramics.name(), "玻璃陶瓷");
    assert_eq!(Industry::Paper.name(), "造紙工業");
    assert_eq!(Industry::Steel.name(), "鋼鐵工業");
    assert_eq!(Industry::Rubber.name(), "橡膠工業");
    assert_eq!(Industry::Automotive.name(), "汽車工業");
    assert_eq!(Industry::Electronic.name(), "電子工業");
    assert_eq!(Industry::ConstructionMaterial.name(), "建材營造業");
    assert_eq!(Industry::Shipping.name(), "航運業");
    assert_eq!(Industry::TourismCatering.name(), "觀光餐旅");
    assert_eq!(Industry::FinanceInsurance.name(), "金融保險業");
    assert_eq!(Industry::TradingDepartmentStores.name(), "貿易百貨");
    assert_eq!(Industry::OilElectricGas.name(), "油電燃氣業");
    assert_eq!(Industry::Comprehensive.name(), "綜合");
    assert_eq!(Industry::Other.name(), "其他");
    assert_eq!(Industry::Chemical.name(), "化學工業");
    assert_eq!(Industry::BiotechMedical.name(), "生技醫療業");
    assert_eq!(Industry::Semiconductor.name(), "半導體業");
    assert_eq!(Industry::ComputerPeripheral.name(), "電腦及週邊設備業");
    assert_eq!(Industry::Optoelectronic.name(), "光電業");
    assert_eq!(Industry::CommunicationNetwork.name(), "通信網路業");
    assert_eq!(Industry::ElectronicComponents.name(), "電子零組件業");
    assert_eq!(Industry::ElectronicPathway.name(), "電子通路業");
    assert_eq!(Industry::InformationService.name(), "資訊服務業");
    assert_eq!(Industry::OtherElectronics.name(), "其他電子業");
    assert_eq!(Industry::CulturalCreative.name(), "文化創意業");
    assert_eq!(Industry::AgriculturalTechnology.name(), "農業科技");
    assert_eq!(Industry::ECommerce.name(), "電子商務");
    assert_eq!(
        Industry::GreenEnergyEnvironmentalProtection.name(),
        "綠能環保"
    );
    assert_eq!(Industry::DigitalCloud.name(), "數位雲端");
    assert_eq!(Industry::SportsRecreation.name(), "運動休閒");
    assert_eq!(Industry::HomeLife.name(), "居家生活");
    assert_eq!(Industry::DepositaryReceipts.name(), "存託憑證");
    assert_eq!(Industry::ExchangeTradedFund.name(), "ETF");
    assert_eq!(Industry::ExchangeTradedNote.name(), "ETN");
    assert_eq!(Industry::BeneficialSecurity.name(), "受益證券");
    assert_eq!(Industry::Uncategorized.name(), "未分類");
}

/// 驗證交易所序號。
#[test]
fn test_stock_exchange_serial_number() {
    assert_eq!(StockExchange::None.serial_number(), 0);
    assert_eq!(StockExchange::TWSE.serial_number(), 1);
    assert_eq!(StockExchange::TPEx.serial_number(), 2);
}

/// 驗證市場別序號。
#[test]
fn test_stock_exchange_market_serial() {
    assert_eq!(StockExchangeMarket::Public.serial(), 1);
    assert_eq!(StockExchangeMarket::Listed.serial(), 2);
    assert_eq!(StockExchangeMarket::OverTheCounter.serial(), 4);
    assert_eq!(StockExchangeMarket::Emerging.serial(), 5);
}

/// 驗證市場別代碼反查。
#[test]
fn test_stock_exchange_market_from() {
    assert_eq!(
        StockExchangeMarket::from(1),
        Some(StockExchangeMarket::Public)
    );
    assert_eq!(
        StockExchangeMarket::from(2),
        Some(StockExchangeMarket::Listed)
    );
    assert_eq!(
        StockExchangeMarket::from(4),
        Some(StockExchangeMarket::OverTheCounter)
    );
    assert_eq!(
        StockExchangeMarket::from(5),
        Some(StockExchangeMarket::Emerging)
    );
    assert_eq!(StockExchangeMarket::from(3), None);
}

#[test]
fn stock_exchange_market_from_rejects_out_of_range_serials() {
    assert_eq!(StockExchangeMarket::from(0), None);
    assert_eq!(StockExchangeMarket::from(-1), None);
    assert_eq!(StockExchangeMarket::from(6), None);
    assert_eq!(StockExchangeMarket::from(9001), None);
}

/// 驗證市場別中文名稱。
#[test]
fn test_stock_exchange_market_name() {
    assert_eq!(StockExchangeMarket::Public.name(), "公開發行");
    assert_eq!(StockExchangeMarket::Listed.name(), "上市");
    assert_eq!(StockExchangeMarket::OverTheCounter.name(), "上櫃");
    assert_eq!(StockExchangeMarket::Emerging.name(), "興櫃");
}

#[test]
fn stock_exchange_market_iterator_matches_serial_lookup() {
    let markets = StockExchangeMarket::iterator().collect::<Vec<_>>();

    assert_eq!(
        markets,
        vec![
            StockExchangeMarket::Public,
            StockExchangeMarket::Listed,
            StockExchangeMarket::OverTheCounter,
            StockExchangeMarket::Emerging,
        ]
    );
    for market in markets {
        assert_eq!(StockExchangeMarket::from(market.serial()), Some(market));
    }
}

#[test]
fn stock_exchange_market_exchange_mapping_is_explicit() {
    assert_eq!(StockExchangeMarket::Public.exchange(), StockExchange::None);
    assert_eq!(StockExchangeMarket::Listed.exchange(), StockExchange::TWSE);
    assert_eq!(
        StockExchangeMarket::OverTheCounter.exchange(),
        StockExchange::TPEx
    );
    assert_eq!(
        StockExchangeMarket::Emerging.exchange(),
        StockExchange::TPEx
    );
}

#[test]
fn stock_exchange_iterator_contains_only_trading_exchanges() {
    assert_eq!(
        StockExchange::iterator().collect::<Vec<_>>(),
        vec![StockExchange::TWSE, StockExchange::TPEx]
    );
}

/// 驗證季度序號。
#[test]
fn test_serial() {
    assert_eq!(Quarter::Q1.serial(), 1);
    assert_eq!(Quarter::Q2.serial(), 2);
    assert_eq!(Quarter::Q3.serial(), 3);
    assert_eq!(Quarter::Q4.serial(), 4);
}

#[test]
fn quarter_iterator_is_ordered_and_round_trips_by_serial() {
    let quarters = Quarter::iterator().collect::<Vec<_>>();

    assert_eq!(
        quarters,
        vec![Quarter::Q1, Quarter::Q2, Quarter::Q3, Quarter::Q4]
    );
    for quarter in quarters {
        assert_eq!(Quarter::from_serial(quarter.serial() as u32), Some(quarter));
    }
}

/// 驗證前一季度計算。
#[test]
fn test_previous() {
    assert_eq!(Quarter::Q1.previous(), Quarter::Q4);
    assert_eq!(Quarter::Q2.previous(), Quarter::Q1);
    assert_eq!(Quarter::Q3.previous(), Quarter::Q2);
    assert_eq!(Quarter::Q4.previous(), Quarter::Q3);
}

/// 驗證月份轉季度。
#[test]
fn test_from_month() {
    assert_eq!(Quarter::from_month(1), Some(Quarter::Q1));
    assert_eq!(Quarter::from_month(3), Some(Quarter::Q1));
    assert_eq!(Quarter::from_month(4), Some(Quarter::Q2));
    assert_eq!(Quarter::from_month(6), Some(Quarter::Q2));
    assert_eq!(Quarter::from_month(7), Some(Quarter::Q3));
    assert_eq!(Quarter::from_month(9), Some(Quarter::Q3));
    assert_eq!(Quarter::from_month(10), Some(Quarter::Q4));
    assert_eq!(Quarter::from_month(12), Some(Quarter::Q4));
    assert_eq!(Quarter::from_month(0), None);
    assert_eq!(Quarter::from_month(13), None);
}

/// 驗證序號轉季度。
#[test]
fn test_from_serial() {
    assert_eq!(Quarter::from_serial(1), Some(Quarter::Q1));
    assert_eq!(Quarter::from_serial(2), Some(Quarter::Q2));
    assert_eq!(Quarter::from_serial(3), Some(Quarter::Q3));
    assert_eq!(Quarter::from_serial(4), Some(Quarter::Q4));
    assert_eq!(Quarter::from_serial(0), None);
    assert_eq!(Quarter::from_serial(5), None);
}

#[test]
fn industry_iterator_contains_each_serial_once() {
    let industries = Industry::iterator().collect::<Vec<_>>();
    let mut serials = industries.iter().map(Industry::serial).collect::<Vec<_>>();
    serials.sort();
    serials.dedup();

    assert_eq!(serials.len(), industries.len());
    assert!(industries.contains(&Industry::Semiconductor));
    assert!(industries.contains(&Industry::ExchangeTradedFund));
    assert!(industries.contains(&Industry::Uncategorized));
}

/// 驗證較早季度清單。
#[test]
fn test_smaller_quarters() {
    assert_eq!(
        Quarter::Q4.smaller_quarters(),
        vec![Quarter::Q1, Quarter::Q2, Quarter::Q3]
    );
    assert_eq!(
        Quarter::Q3.smaller_quarters(),
        vec![Quarter::Q1, Quarter::Q2]
    );
    assert_eq!(Quarter::Q2.smaller_quarters(), vec![Quarter::Q1]);
    assert_eq!(Quarter::Q1.smaller_quarters(), vec![]);
}
