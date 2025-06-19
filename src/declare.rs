use chrono::{Local, NaiveTime};
use serde_derive::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

#[derive(
    Serialize, Deserialize, Display, Debug, Copy, Clone, EnumString, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum Quarter {
    #[strum(serialize = "Q1")]
    Q1 = 1,
    #[strum(serialize = "Q2")]
    Q2 = 2,
    #[strum(serialize = "Q3")]
    Q3 = 3,
    #[strum(serialize = "Q4")]
    Q4 = 4,
}

impl Quarter {
    /// Returns the serial number of the quarter as i32.
    pub fn serial(&self) -> i32 {
        *self as i32
    }

    /// Returns the previous quarter.
    pub fn previous(&self) -> Quarter {
        match self {
            Quarter::Q1 => Quarter::Q4,
            Quarter::Q2 => Quarter::Q1,
            Quarter::Q3 => Quarter::Q2,
            Quarter::Q4 => Quarter::Q3,
        }
    }

    /// Returns the quarter corresponding to a given month.
    pub fn from_month(month: u32) -> Option<Quarter> {
        match month {
            1..=3 => Some(Quarter::Q1),
            4..=6 => Some(Quarter::Q2),
            7..=9 => Some(Quarter::Q3),
            10..=12 => Some(Quarter::Q4),
            _ => None,
        }
    }

    /// Returns the quarter corresponding to a given serial number.
    pub fn from_serial(val: u32) -> Option<Quarter> {
        match val {
            1 => Some(Quarter::Q1),
            2 => Some(Quarter::Q2),
            3 => Some(Quarter::Q3),
            4 => Some(Quarter::Q4),
            _ => None,
        }
    }

    /// Returns an iterator over the quarters.
    pub fn iterator() -> impl Iterator<Item = Self> {
        [Quarter::Q1, Quarter::Q2, Quarter::Q3, Quarter::Q4]
            .iter()
            .copied()
    }

    /// Returns a vector of `Quarter` values that are smaller than the current quarter.
    ///
    /// # Examples
    ///
    /// ```
    /// let q4 = Quarter::Q4;
    /// let smaller_quarters = q4.smaller_quarters();
    /// assert_eq!(smaller_quarters, vec![Quarter::Q1, Quarter::Q2, Quarter::Q3]);
    /// ```
    pub fn smaller_quarters(&self) -> Vec<Quarter> {
        Self::iterator().take_while(|&q| q < *self).collect()
    }
}

/// 交易所
#[derive(Debug, Copy, Clone, Display, PartialEq, Serialize, Deserialize, EnumString)]
pub enum StockExchange {
    /// 未有交易所
    None = 0,
    /// 臺灣證券交易所 1
    #[strum(serialize = "臺灣證券交易所")]
    TWSE = 1,
    /// 證券櫃檯買賣市場 2
    #[strum(serialize = "中華民國證券櫃檯買賣中心")]
    TPEx = 2,
}

impl StockExchange {
    /// 返回交易所的序列號
    pub fn serial_number(&self) -> i32 {
        *self as i32
    }

    /// 目前的時間是否為開盤時間
    pub fn is_open(&self) -> bool {
        // 獲取當前時間
        let now = Local::now().time();
        let start_time = NaiveTime::from_hms_opt(9, 0, 0).expect("Invalid start time");
        let end_time = NaiveTime::from_hms_opt(13, 30, 0).expect("Invalid end time");

        // 判斷當前時間是否在範圍內
        now >= start_time && now <= end_time
    }

    pub fn iterator() -> impl Iterator<Item = Self> {
        [Self::TWSE, Self::TPEx].iter().copied()
    }
}

/// 市場別
#[derive(PartialEq, Debug, Copy, Clone, Display, EnumString)]
#[repr(i32)]
#[non_exhaustive]
pub enum StockExchangeMarket {
    /// 公開發行
    #[strum(serialize = "公開發行")]
    Public = 1,
    /// 上市 2
    #[strum(serialize = "上市")]
    Listed = 2,
    /// 上櫃 4
    #[strum(serialize = "上櫃")]
    OverTheCounter = 4,
    /// 興櫃 5
    #[strum(serialize = "興櫃")]
    Emerging = 5,
}

impl StockExchangeMarket {
    /// 返回市場的序列號
    pub fn serial(&self) -> i32 {
        *self as i32
    }

    /// 根據序列號返回對應的市場
    pub fn from(serial: i32) -> Option<StockExchangeMarket> {
        match serial {
            1 => Some(StockExchangeMarket::Public),
            2 => Some(StockExchangeMarket::Listed),
            4 => Some(StockExchangeMarket::OverTheCounter),
            5 => Some(StockExchangeMarket::Emerging),
            _ => None,
        }
    }

    /// 返回市場的名稱
    pub fn name(&self) -> String {
        self.to_string()
    }

    /// 返回市場對應的交易所
    pub fn exchange(&self) -> StockExchange {
        match self {
            StockExchangeMarket::Listed => StockExchange::TWSE,
            StockExchangeMarket::OverTheCounter | StockExchangeMarket::Emerging => {
                StockExchange::TPEx
            }
            StockExchangeMarket::Public => StockExchange::None,
        }
    }

    /// 返回市場的迭代器
    pub fn iterator() -> impl Iterator<Item = Self> {
        [
            Self::Public,
            Self::Listed,
            Self::OverTheCounter,
            Self::Emerging,
        ]
        .iter()
        .copied()
    }
}

/// 產業分類
#[derive(PartialEq, Debug, Copy, Clone, Display, EnumString)]
#[repr(i32)]
pub enum Industry {
    /// 水泥工業 1
    #[strum(serialize = "水泥工業")]
    Cement = 1,
    /// 食品工業 2
    #[strum(serialize = "食品工業")]
    Food = 2,
    /// 塑膠工業 3
    #[strum(serialize = "塑膠工業")]
    Plastic = 3,
    /// 紡織纖維 4
    #[strum(serialize = "紡織纖維")]
    TextileFiber = 4,
    /// 電機機械 5
    #[strum(serialize = "電機機械")]
    ElectricalMachinery = 5,
    /// 電器電纜 6
    #[strum(serialize = "電器電纜")]
    ElectricalCable = 6,
    /// 玻璃陶瓷 8
    #[strum(serialize = "玻璃陶瓷")]
    GlassCeramics = 8,
    /// 造紙工業 9
    #[strum(serialize = "造紙工業")]
    Paper = 9,
    /// 鋼鐵工業 10
    #[strum(serialize = "鋼鐵工業")]
    Steel = 10,
    /// 橡膠工業 11
    #[strum(serialize = "橡膠工業")]
    Rubber = 11,
    /// 汽車工業 12
    #[strum(serialize = "汽車工業")]
    Automotive = 12,
    /// 電子工業 13
    #[strum(serialize = "電子工業")]
    Electronic = 13,
    /// 建材營造業 14
    #[strum(serialize = "建材營造業")]
    ConstructionMaterial = 14,
    /// 航運業 15
    #[strum(serialize = "航運業")]
    Shipping = 15,
    /// 觀光餐旅 16
    #[strum(serialize = "觀光餐旅")]
    TourismCatering = 16,
    /// 金融保險業 17
    #[strum(serialize = "金融保險業")]
    FinanceInsurance = 17,
    /// 貿易百貨業 18
    #[strum(serialize = "貿易百貨")]
    TradingDepartmentStores = 18,
    /// 綜合 19
    #[strum(serialize = "綜合")]
    Comprehensive = 19,
    /// 其他業 20
    #[strum(serialize = "其他")]
    Other = 20,
    /// 化學工業 21
    #[strum(serialize = "化學工業")]
    Chemical = 21,
    /// 生技醫療業 22
    #[strum(serialize = "生技醫療業")]
    BiotechMedical = 22,
    /// 油電燃氣業 23
    #[strum(serialize = "油電燃氣業")]
    OilElectricGas = 23,
    /// 半導體業 24
    #[strum(serialize = "半導體業")]
    Semiconductor = 24,
    /// 電腦及週邊設備業 25
    #[strum(serialize = "電腦及週邊設備業")]
    ComputerPeripheral = 25,
    /// 光電業 26
    #[strum(serialize = "光電業")]
    Optoelectronic = 26,
    /// 通信網路業 27
    #[strum(serialize = "通信網路業")]
    CommunicationNetwork = 27,
    /// 電子零組件業 28
    #[strum(serialize = "電子零組件業")]
    ElectronicComponents = 28,
    /// 電子通路業 29
    #[strum(serialize = "電子通路業")]
    ElectronicPathway = 29,
    /// 資訊服務業 30
    #[strum(serialize = "資訊服務業")]
    InformationService = 30,
    /// 其他電子業 31
    #[strum(serialize = "其他電子業")]
    OtherElectronics = 31,
    /// 文化創意業 32
    #[strum(serialize = "文化創意業")]
    CulturalCreative = 32,
    /// 農業科技 33
    #[strum(serialize = "農業科技")]
    AgriculturalTechnology = 33,
    /// 電子商務 34
    #[strum(serialize = "電子商務")]
    ECommerce = 34,
    /// 綠能環保 35
    #[strum(serialize = "綠能環保")]
    GreenEnergyEnvironmentalProtection = 35,
    /// 數位雲端 36
    #[strum(serialize = "數位雲端")]
    DigitalCloud = 36,
    /// 運動休閒 37
    #[strum(serialize = "運動休閒")]
    SportsRecreation = 37,
    /// 居家生活 38
    #[strum(serialize = "居家生活")]
    HomeLife = 38,
    /// 存託憑證 39
    #[strum(serialize = "存託憑證")]
    DepositaryReceipts = 39,
    /// 未分類 99
    #[strum(serialize = "未分類")]
    Uncategorized = 99,
}
impl Industry {
    pub fn serial(&self) -> i32 {
        *self as i32
    }

    pub fn name(&self) -> String {
        self.to_string()
    }

    pub fn iterator() -> impl Iterator<Item = Self> {
        [
            Self::Cement,
            Self::Food,
            Self::Plastic,
            Self::TextileFiber,
            Self::ElectricalMachinery,
            Self::ElectricalCable,
            Self::Chemical,
            Self::BiotechMedical,
            Self::GlassCeramics,
            Self::Paper,
            Self::Steel,
            Self::Rubber,
            Self::Automotive,
            Self::Semiconductor,
            Self::ComputerPeripheral,
            Self::Optoelectronic,
            Self::CommunicationNetwork,
            Self::ElectronicComponents,
            Self::ElectronicPathway,
            Self::InformationService,
            Self::OtherElectronics,
            Self::ConstructionMaterial,
            Self::Shipping,
            Self::FinanceInsurance,
            Self::TradingDepartmentStores,
            Self::OilElectricGas,
            Self::Comprehensive,
            Self::GreenEnergyEnvironmentalProtection,
            Self::DigitalCloud,
            Self::SportsRecreation,
            Self::HomeLife,
            Self::Other,
            Self::CulturalCreative,
            Self::AgriculturalTechnology,
            Self::ECommerce,
            Self::TourismCatering,
            Self::DepositaryReceipts,
            Self::Uncategorized,
        ]
        .iter()
        .copied()
    }
}

/// 股票報價
#[derive(Debug)]
pub struct StockQuotes {
    pub stock_symbol: String,
    pub price: f64,
    /// 漲跌
    pub change: f64,
    /// 漲跌百分比
    pub change_range: f64,
}

/// 三天的秒數
pub const THREE_DAYS_IN_SECONDS: usize = 60 * 60 * 24 * 3;
/// 一天的秒數
pub const ONE_DAYS_IN_SECONDS: usize = 60 * 60 * 24;

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(Industry::Uncategorized.serial(), 99);
    }

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
        assert_eq!(Industry::Uncategorized.name(), "未分類");
    }

    #[test]
    fn test_stock_exchange_serial_number() {
        assert_eq!(StockExchange::None.serial_number(), 0);
        assert_eq!(StockExchange::TWSE.serial_number(), 1);
        assert_eq!(StockExchange::TPEx.serial_number(), 2);
    }

    #[test]
    fn test_stock_exchange_market_serial() {
        assert_eq!(StockExchangeMarket::Public.serial(), 1);
        assert_eq!(StockExchangeMarket::Listed.serial(), 2);
        assert_eq!(StockExchangeMarket::OverTheCounter.serial(), 4);
        assert_eq!(StockExchangeMarket::Emerging.serial(), 5);
    }

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
    fn test_stock_exchange_market_name() {
        assert_eq!(StockExchangeMarket::Public.name(), "公開發行");
        assert_eq!(StockExchangeMarket::Listed.name(), "上市");
        assert_eq!(StockExchangeMarket::OverTheCounter.name(), "上櫃");
        assert_eq!(StockExchangeMarket::Emerging.name(), "興櫃");
    }

    #[test]
    fn test_serial() {
        assert_eq!(Quarter::Q1.serial(), 1);
        assert_eq!(Quarter::Q2.serial(), 2);
        assert_eq!(Quarter::Q3.serial(), 3);
        assert_eq!(Quarter::Q4.serial(), 4);
    }

    #[test]
    fn test_previous() {
        assert_eq!(Quarter::Q1.previous(), Quarter::Q4);
        assert_eq!(Quarter::Q2.previous(), Quarter::Q1);
        assert_eq!(Quarter::Q3.previous(), Quarter::Q2);
        assert_eq!(Quarter::Q4.previous(), Quarter::Q3);
    }

    #[test]
    fn test_from_month() {
        assert_eq!(Quarter::from_month(1), Some(Quarter::Q1));
        assert_eq!(Quarter::from_month(4), Some(Quarter::Q2));
        assert_eq!(Quarter::from_month(7), Some(Quarter::Q3));
        assert_eq!(Quarter::from_month(10), Some(Quarter::Q4));
        assert_eq!(Quarter::from_month(13), None);
    }

    #[test]
    fn test_from_serial() {
        assert_eq!(Quarter::from_serial(1), Some(Quarter::Q1));
        assert_eq!(Quarter::from_serial(2), Some(Quarter::Q2));
        assert_eq!(Quarter::from_serial(3), Some(Quarter::Q3));
        assert_eq!(Quarter::from_serial(4), Some(Quarter::Q4));
        assert_eq!(Quarter::from_serial(5), None);
    }

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
}
