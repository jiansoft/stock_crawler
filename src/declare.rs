use chrono::{Local, NaiveTime};
use serde_derive::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

#[derive(
    Serialize,
    Deserialize,
    Display,
    Debug,
    Copy,
    Clone,
    EnumString,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
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
        [Quarter::Q1, Quarter::Q2, Quarter::Q3, Quarter::Q4].iter().copied()
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
#[derive(Debug, Copy, Clone)]
pub enum StockExchange {
    /// 未有交易所
    None,
    /// 臺灣證券交易所 1
    TWSE,
    /// 證券櫃檯買賣市場 2
    TPEx,
}

impl StockExchange {
    pub fn serial_number(&self) -> i32 {
        match self {
            StockExchange::None => 0,
            StockExchange::TWSE => 1,
            StockExchange::TPEx => 2,
        }
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
#[derive(PartialEq, Debug, Copy, Clone)]
#[repr(i32)]
#[non_exhaustive]
pub enum StockExchangeMarket {
    /// 公開發行
    Public = 1,
    /// 上市 2
    Listed = 2,
    /// 上櫃 4
    OverTheCounter = 4,
    /// 興櫃 5
    Emerging = 5,
}

impl StockExchangeMarket {
    pub fn serial(&self) -> i32 {
        *self as i32
    }

    pub fn from(serial: i32) -> Option<StockExchangeMarket> {
        match serial {
            x if x == StockExchangeMarket::Listed.serial() => Some(StockExchangeMarket::Listed),
            x if x == StockExchangeMarket::OverTheCounter.serial() => {
                Some(StockExchangeMarket::OverTheCounter)
            }
            x if x == StockExchangeMarket::Emerging.serial() => Some(StockExchangeMarket::Emerging),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match *self {
            StockExchangeMarket::Public => "公開發行",
            StockExchangeMarket::Listed => "上市",
            StockExchangeMarket::OverTheCounter => "上櫃",
            StockExchangeMarket::Emerging => "興櫃",
        }
    }

    pub fn exchange(&self) -> StockExchange {
        match self {
            StockExchangeMarket::Listed => StockExchange::TWSE,
            StockExchangeMarket::OverTheCounter | StockExchangeMarket::Emerging => {
                StockExchange::TPEx
            }
            StockExchangeMarket::Public => StockExchange::None,
        }
    }

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
#[derive(PartialEq, Debug, Copy, Clone)]
#[repr(i32)]
pub enum Industry {
    /// 水泥工業 1
    Cement = 1,
    /// 食品工業 2
    Food = 2,
    /// 塑膠工業 3
    Plastic = 3,
    /// 紡織纖維 4
    TextileFiber = 4,
    /// 電機機械 5
    ElectricalMachinery = 5,
    /// 電器電纜 6
    ElectricalCable = 6,
    /// 玻璃陶瓷 8
    GlassCeramics = 8,
    /// 造紙工業 9
    Paper = 9,
    /// 鋼鐵工業 10
    Steel = 10,
    /// 橡膠工業 11
    Rubber = 11,
    /// 汽車工業 12
    Automotive = 12,
    /// 電子工業 13
    Electronic = 13,
    /// 建材營造業 14
    ConstructionMaterial = 14,
    /// 航運業 15
    Shipping = 15,
    /// 觀光餐旅 16
    TourismCatering = 16,
    /// 金融保險業 17
    FinanceInsurance = 17,
    /// 貿易百貨業 18
    TradingDepartmentStores = 18,
    /// 綜合 19
    Comprehensive = 19,
    /// 其他業 20
    Other = 20,
    /// 化學工業 21
    Chemical = 21,
    /// 生技醫療業 22
    BiotechMedical = 22,
    /// 油電燃氣業 23
    OilElectricGas = 23,
    /// 半導體業 24
    Semiconductor = 24,
    /// 電腦及週邊設備業 25
    ComputerPeripheral = 25,
    /// 光電業 26
    Optoelectronic = 26,
    /// 通信網路業 27
    CommunicationNetwork = 27,
    /// 電子零組件業 28
    ElectronicComponents = 28,
    /// 電子通路業 29
    ElectronicPathway = 29,
    /// 資訊服務業 30
    InformationService = 30,
    /// 其他電子業 31
    OtherElectronics = 31,
    /// 文化創意業 32
    CulturalCreative = 32,
    /// 農業科技 33
    AgriculturalTechnology = 33,
    /// 電子商務 34
    ECommerce = 34,
    /// 綠能環保 35
    GreenEnergyEnvironmentalProtection = 35,
    /// 數位雲端 36
    DigitalCloud = 36,
    /// 運動休閒 37
    SportsRecreation = 37,
    /// 居家生活 38
    HomeLife = 38,
    /// 存託憑證 39
    DepositaryReceipts = 39,
    /// 未分類 99
    Uncategorized = 99,
}

impl Industry {
    pub fn serial(&self) -> i32 {
        *self as i32
    }

    pub fn name(&self) -> &'static str {
        match *self {
            Industry::Cement => "水泥工業",
            Industry::Food => "食品工業",
            Industry::Plastic => "塑膠工業",
            Industry::TextileFiber => "紡織纖維",
            Industry::ElectricalMachinery => "電機機械",
            Industry::ElectricalCable => "電器電纜",
            Industry::Chemical => "化學工業",
            Industry::BiotechMedical => "生技醫療業",
            Industry::GlassCeramics => "玻璃陶瓷",
            Industry::Paper => "造紙工業",
            Industry::Steel => "鋼鐵工業",
            Industry::Rubber => "橡膠工業",
            Industry::Automotive => "汽車工業",
            Industry::Semiconductor => "半導體業",
            Industry::ComputerPeripheral => "電腦及週邊設備業",
            Industry::Optoelectronic => "光電業",
            Industry::CommunicationNetwork => "通信網路業",
            Industry::ElectronicComponents => "電子零組件業",
            Industry::ElectronicPathway => "電子通路業",
            Industry::InformationService => "資訊服務業",
            Industry::OtherElectronics => "其他電子業",
            Industry::ConstructionMaterial => "建材營造業",
            Industry::Shipping => "航運業",
            Industry::FinanceInsurance => "金融保險業",
            Industry::TradingDepartmentStores => "貿易百貨",
            Industry::OilElectricGas => "油電燃氣業",
            Industry::Comprehensive => "綜合",
            Industry::GreenEnergyEnvironmentalProtection => "綠能環保",
            Industry::DigitalCloud => "數位雲端",
            Industry::SportsRecreation => "運動休閒",
            Industry::HomeLife => "居家生活",
            Industry::Other => "其他",
            Industry::CulturalCreative => "文化創意業",
            Industry::AgriculturalTechnology => "農業科技",
            Industry::ECommerce => "電子商務",
            Industry::TourismCatering => "觀光餐旅",
            Industry::DepositaryReceipts => "存託憑證",
            Industry::Uncategorized => "未分類",
            Industry::Electronic => "電子工業",
        }
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
    use crate::declare::Quarter;

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
        assert_eq!(Quarter::Q4.smaller_quarters(), vec![Quarter::Q1, Quarter::Q2, Quarter::Q3]);
        assert_eq!(Quarter::Q3.smaller_quarters(), vec![Quarter::Q1, Quarter::Q2]);
        assert_eq!(Quarter::Q2.smaller_quarters(), vec![Quarter::Q1]);
        assert_eq!(Quarter::Q1.smaller_quarters(), vec![]);
    }
}
