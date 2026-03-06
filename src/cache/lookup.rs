//! 快取所需的靜態對照表。
//!
//! 此模組集中建立不需要 `RwLock` 保護的固定查表資料，
//! 例如交易市場代碼與產業名稱對照。

use std::collections::HashMap;

use crate::{
    database::table::stock_exchange_market,
    declare::{self, Industry},
};

/// 建立預設的交易市場代碼對照表。
///
/// # 回傳
/// - `HashMap<i32, StockExchangeMarket>`：以市場代碼為 key，
///   對應上市、上櫃、興櫃等市場資訊。
pub(crate) fn default_exchange_markets() -> HashMap<i32, stock_exchange_market::StockExchangeMarket>
{
    // 這些對照表屬於靜態基礎資料，程式啟動後通常不會變動。
    HashMap::from([
        (
            2,
            stock_exchange_market::StockExchangeMarket {
                stock_exchange_market_id: 2,
                stock_exchange_id: 1,
                code: "TAI".to_string(),
                name: declare::StockExchangeMarket::Listed.name(),
            },
        ),
        (
            4,
            stock_exchange_market::StockExchangeMarket {
                stock_exchange_market_id: 4,
                stock_exchange_id: 2,
                code: "TWO".to_string(),
                name: declare::StockExchangeMarket::OverTheCounter.name(),
            },
        ),
        (
            5,
            stock_exchange_market::StockExchangeMarket {
                stock_exchange_market_id: 5,
                stock_exchange_id: 2,
                code: "TWE".to_string(),
                name: declare::StockExchangeMarket::Emerging.name(),
            },
        ),
    ])
}

/// 建立預設的產業名稱對照表。
///
/// # 回傳
/// - `HashMap<String, i32>`：以產業中文名稱為 key、產業代碼為 value。
///
/// # 實作說明
/// - 包含正式產業名稱。
/// - 也包含少量歷史名稱或同義名稱，方便資料來源格式不一致時仍可正確映射。
pub(crate) fn default_industries() -> HashMap<String, i32> {
    HashMap::from([
        (Industry::Cement.name(), Industry::Cement.serial()),
        (Industry::Food.name(), Industry::Food.serial()),
        (Industry::Plastic.name(), Industry::Plastic.serial()),
        (
            Industry::TextileFiber.name(),
            Industry::TextileFiber.serial(),
        ),
        (
            Industry::ElectricalMachinery.name(),
            Industry::ElectricalMachinery.serial(),
        ),
        (
            Industry::ElectricalCable.name(),
            Industry::ElectricalCable.serial(),
        ),
        (Industry::Chemical.name(), Industry::Chemical.serial()),
        (
            Industry::BiotechMedical.name(),
            Industry::BiotechMedical.serial(),
        ),
        (
            Industry::GlassCeramics.name(),
            Industry::GlassCeramics.serial(),
        ),
        (Industry::Paper.name(), Industry::Paper.serial()),
        (Industry::Steel.name(), Industry::Steel.serial()),
        (Industry::Rubber.name(), Industry::Rubber.serial()),
        (Industry::Automotive.name(), Industry::Automotive.serial()),
        (
            Industry::Semiconductor.name(),
            Industry::Semiconductor.serial(),
        ),
        (
            Industry::ComputerPeripheral.name(),
            Industry::ComputerPeripheral.serial(),
        ),
        (
            Industry::Optoelectronic.name(),
            Industry::Optoelectronic.serial(),
        ),
        (
            Industry::CommunicationNetwork.name(),
            Industry::CommunicationNetwork.serial(),
        ),
        (
            Industry::ElectronicComponents.name(),
            Industry::ElectronicComponents.serial(),
        ),
        (
            Industry::ElectronicPathway.name(),
            Industry::ElectronicPathway.serial(),
        ),
        (
            Industry::InformationService.name(),
            Industry::InformationService.serial(),
        ),
        (
            Industry::OtherElectronics.name(),
            Industry::OtherElectronics.serial(),
        ),
        (
            Industry::ConstructionMaterial.name(),
            Industry::ConstructionMaterial.serial(),
        ),
        (Industry::Shipping.name(), Industry::Shipping.serial()),
        (
            Industry::FinanceInsurance.name(),
            Industry::FinanceInsurance.serial(),
        ),
        (
            Industry::TradingDepartmentStores.name(),
            Industry::TradingDepartmentStores.serial(),
        ),
        (
            Industry::OilElectricGas.name(),
            Industry::OilElectricGas.serial(),
        ),
        (
            Industry::Comprehensive.name(),
            Industry::Comprehensive.serial(),
        ),
        (
            Industry::GreenEnergyEnvironmentalProtection.name(),
            Industry::GreenEnergyEnvironmentalProtection.serial(),
        ),
        (
            Industry::DigitalCloud.name(),
            Industry::DigitalCloud.serial(),
        ),
        (
            Industry::SportsRecreation.name(),
            Industry::SportsRecreation.serial(),
        ),
        (Industry::HomeLife.name(), Industry::HomeLife.serial()),
        (Industry::Other.name(), Industry::Other.serial()),
        (
            Industry::CulturalCreative.name(),
            Industry::CulturalCreative.serial(),
        ),
        (
            Industry::AgriculturalTechnology.name(),
            Industry::AgriculturalTechnology.serial(),
        ),
        (Industry::ECommerce.name(), Industry::ECommerce.serial()),
        (
            Industry::TourismCatering.name(),
            Industry::TourismCatering.serial(),
        ),
        (
            Industry::DepositaryReceipts.name(),
            Industry::DepositaryReceipts.serial(),
        ),
        (
            Industry::Uncategorized.name(),
            Industry::Uncategorized.serial(),
        ),
        (
            "貿易百貨業".to_string(),
            Industry::TradingDepartmentStores.serial(),
        ),
        ("其他業".to_string(), Industry::Other.serial()),
        (
            "農業科技業".to_string(),
            Industry::AgriculturalTechnology.serial(),
        ),
    ])
}
