/// 交易所
#[derive(Debug, Copy, Clone)]
pub enum StockExchange {
    /// 臺灣證券交易所 1
    TWSE,
    /// 證券櫃檯買賣市場 2
    TPEx,
}

impl StockExchange {
    pub fn serial_number(&self) -> i32 {
        match self {
            StockExchange::TWSE => 1,
            StockExchange::TPEx => 2,
        }
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
        }
    }

    pub fn iterator() -> impl Iterator<Item = Self> {
        [Self::Listed, Self::OverTheCounter, Self::Emerging]
            .iter()
            .copied()
    }
}

///產業分類
#[derive(PartialEq, Debug, Copy, Clone)]
#[repr(i32)]
pub enum Industry {
    /// 水泥工業 1
    CementIndustry = 1,
    /// 食品工業 2
    FoodIndustry = 2,
    /// 塑膠工業 3
    PlasticIndustry = 3,
    /// 紡織纖維 4
    TextileFiber = 4,
    /// 電機機械 5
    ElectricalMachinery = 5,
    /// 電器電纜 6
    ElectricalCable = 6,
    /// 化學工業 7
    ChemicalIndustry = 7,
    /// 生技醫療業 8
    BiotechMedical = 8,
    /// 玻璃陶瓷 9
    GlassCeramics = 9,
    /// 造紙工業 10
    PaperIndustry = 10,
    /// 鋼鐵工業 11
    SteelIndustry = 11,
    /// 橡膠工業 12
    RubberIndustry = 12,
    /// 汽車工業 13
    AutomotiveIndustry = 13,
    /// 半導體業 14
    Semiconductor = 14,
    /// 電腦及週邊設備業 15
    ComputerPeripheral = 15,
    /// 光電業 16
    Optoelectronic = 16,
    /// 通訊網路業 17
    CommunicationNetwork = 17,
    /// 電子零組件業 18
    ElectronicComponents = 18,
    /// 電子通路業 19
    ElectronicPathway = 19,
    /// 資訊服務業 20
    InformationService = 20,
    /// 其他電子業 21
    OtherElectronics = 21,
    /// 建材營造業 22
    ConstructionMaterial = 22,
    /// 航運業 23
    Shipping = 23,
    /// 觀光事業 24
    Tourism = 24,
    /// 金融保險業 25
    FinanceInsurance = 25,
    /// 貿易百貨 26
    TradingDepartmentStores = 26,
    /// 貿易百貨業 26
    //TradingDepartmentStoresIndustry = 26,
    /// 油電燃氣業 27
    OilElectricGas = 27,
    /// 綜合 28
    Comprehensive = 28,
    /// 綠能環保 29
    GreenEnergyEnvironmentalProtection = 29,
    /// 數位雲端 30
    DigitalCloud = 30,
    /// 運動休閒 31
    SportsRecreation = 31,
    /// 居家生活 32
    HomeLife = 32,
    /// 其他 33
    Other = 33,
    /// 其他業 33
    //OtherIndustry = 33,
    /// 文化創意業 34
    CulturalCreative = 34,
    /// 農業科技 35
    AgriculturalTechnology = 35,
    /// 農業科技業 35
    //AgriculturalTechnologyIndustry = 35,
    /// 電子商務 36
    ECommerce = 36,
    /// 觀光餐旅 37
    TourismCatering = 37,
    /// 存託憑證 38
    DepositaryReceipts = 38,
    /// 未分類 99
    Uncategorized = 99,
}

#[cfg(test)]
mod tests {}
