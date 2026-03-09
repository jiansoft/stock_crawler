//! # Yahoo 財經採集模組
//!
//! 此模組專門負責從 Yahoo 財經（台灣站）抓取各類證券資料。
//!
//! ## 支援的功能
//!
//! - **即時報價 (`price`)**：抓取最新成交價、漲跌幅、開盤、最高、最低價等。
//! - **基本面資料 (`profile`)**：抓取毛利率、營益率、ROE、ROA、EPS 等財務比率。
//! - **股利政策 (`dividend`)**：抓取歷年現金股利、股票股利、除息日及發放日明細。
//!
//! ## 站點資訊
//!
//! - 來源域名：`tw.stock.yahoo.com`
//! - 抓取技術：HTTP GET 搭配 CSS Selector 解析。

/// 股利數據採集子模組
pub mod dividend;
/// 即時報價與行情採集子模組
pub mod price;
/// 財務比率與基本面資料採集子模組
pub mod profile;

/// Yahoo 財經台灣站的主機域名
const HOST: &str = "tw.stock.yahoo.com";

/// Yahoo 台股類股頁面。
pub const CLASS_URL: &str = "https://tw.stock.yahoo.com/class";

/// Yahoo 台股類股所使用的交易所代碼。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YahooClassExchange {
    /// 上市類股。
    Listed,
    /// 上櫃類股。
    OverTheCounter,
    /// 興櫃類股。
    Emerging,
}

impl YahooClassExchange {
    /// 取得 Yahoo 使用的交易所代碼。
    pub const fn code(self) -> &'static str {
        match self {
            Self::Listed => "TAI",
            Self::OverTheCounter => "TWO",
            Self::Emerging => "OES",
        }
    }

    /// 取得中文名稱。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Listed => "上市",
            Self::OverTheCounter => "上櫃",
            Self::Emerging => "興櫃",
        }
    }
}

/// Yahoo 類股分類定義。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YahooClassCategory {
    /// 類股所屬交易所。
    pub exchange: YahooClassExchange,
    /// Yahoo `class-quote` 頁面的 `sectorId`。
    pub sector_id: u32,
    /// 類股中文名稱。
    pub name: &'static str,
}

/// Yahoo 上市類股分類。
pub const LISTED_CLASS_CATEGORIES: &[YahooClassCategory] = &[
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 1,
        name: "水泥",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 2,
        name: "食品",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 3,
        name: "塑膠",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 4,
        name: "紡織",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 6,
        name: "電機機械",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 7,
        name: "電器電纜",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 9,
        name: "玻璃",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 10,
        name: "造紙",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 11,
        name: "鋼鐵",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 12,
        name: "橡膠",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 13,
        name: "汽車",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 19,
        name: "營建",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 20,
        name: "航運",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 21,
        name: "觀光餐旅",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 22,
        name: "金融業",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 24,
        name: "貿易百貨",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 25,
        name: "存託憑證",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 26,
        name: "ETF",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 29,
        name: "受益證券",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 30,
        name: "其他",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 31,
        name: "市認購",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 32,
        name: "市認售",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 33,
        name: "指數類",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 37,
        name: "化學",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 38,
        name: "生技",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 39,
        name: "油電燃氣",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 40,
        name: "半導體",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 41,
        name: "電腦週邊",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 42,
        name: "光電",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 43,
        name: "通訊網路",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 44,
        name: "電子零組件",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 45,
        name: "電子通路",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 46,
        name: "資訊服務",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 47,
        name: "其他電子",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 48,
        name: "ETN",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 49,
        name: "創新板",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 51,
        name: "市牛證",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 52,
        name: "市熊證",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 93,
        name: "綠能環保",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 94,
        name: "數位雲端",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 95,
        name: "運動休閒",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Listed,
        sector_id: 96,
        name: "居家生活",
    },
];

/// Yahoo 上櫃類股分類。
pub const OTC_CLASS_CATEGORIES: &[YahooClassCategory] = &[
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 121,
        name: "生技",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 122,
        name: "食品",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 123,
        name: "塑膠",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 124,
        name: "紡織",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 125,
        name: "電機",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 126,
        name: "電器電纜",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 130,
        name: "鋼鐵",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 138,
        name: "營建",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 139,
        name: "航運",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 140,
        name: "觀光餐旅",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 141,
        name: "金融",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 142,
        name: "居家生活",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 145,
        name: "其他",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 151,
        name: "化學",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 153,
        name: "半導體",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 154,
        name: "電腦週邊",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 155,
        name: "光電",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 156,
        name: "通訊網路",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 157,
        name: "電子零組件",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 158,
        name: "電子通路",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 159,
        name: "資訊服務",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 160,
        name: "其他電子",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 161,
        name: "油電燃氣",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 163,
        name: "公司債",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 165,
        name: "認購",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 166,
        name: "認售",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 167,
        name: "牛證",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 169,
        name: "文化創意",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 170,
        name: "農業科技業",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 171,
        name: "電子商務",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 172,
        name: "ETF",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 173,
        name: "ETN",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 97,
        name: "綠能環保",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 98,
        name: "運動休閒",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::OverTheCounter,
        sector_id: 33,
        name: "指數類",
    },
];

/// Yahoo 興櫃類股分類。
pub const EMERGING_CLASS_CATEGORIES: &[YahooClassCategory] = &[
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99301,
        name: "食品",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99303,
        name: "紡織",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99304,
        name: "電機",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99306,
        name: "化學工業",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99307,
        name: "生技",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99309,
        name: "鋼鐵",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99311,
        name: "半導體",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99312,
        name: "電腦週邊",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99313,
        name: "光電",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99314,
        name: "通信網路",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99315,
        name: "電子零組件",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99316,
        name: "電子通路",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99317,
        name: "資訊服務",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99318,
        name: "其他電子",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99319,
        name: "營建",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99320,
        name: "航運",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99321,
        name: "觀光餐旅",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99322,
        name: "金融",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99323,
        name: "居家生活",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99324,
        name: "油電燃氣",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99325,
        name: "其他",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99327,
        name: "文化創意",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99328,
        name: "基金黃金",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99329,
        name: "農業科技業",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99330,
        name: "數位雲端",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99331,
        name: "綠能環保",
    },
    YahooClassCategory {
        exchange: YahooClassExchange::Emerging,
        sector_id: 99332,
        name: "運動休閒",
    },
];

/// 依交易所取得 Yahoo 類股分類。
pub const fn class_categories(exchange: YahooClassExchange) -> &'static [YahooClassCategory] {
    match exchange {
        YahooClassExchange::Listed => LISTED_CLASS_CATEGORIES,
        YahooClassExchange::OverTheCounter => OTC_CLASS_CATEGORIES,
        YahooClassExchange::Emerging => EMERGING_CLASS_CATEGORIES,
    }
}

/// Yahoo 財經採集器
///
/// 此結構體主要作為 `StockInfo` Trait 的實作載體，提供統一的採集介面。
pub struct Yahoo {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 依 `exchange + sector_id` 從預先整理的類股清單中找出目標類股。
    fn find_category(
        exchange: YahooClassExchange,
        sector_id: u32,
    ) -> Option<&'static YahooClassCategory> {
        class_categories(exchange)
            .iter()
            .find(|category| category.sector_id == sector_id)
    }

    /// 驗證三大市場的 Yahoo 交易所代碼與中文標籤是否正確。
    #[test]
    fn test_exchange_code_and_label() {
        assert_eq!(YahooClassExchange::Listed.code(), "TAI");
        assert_eq!(YahooClassExchange::Listed.label(), "上市");
        assert_eq!(YahooClassExchange::OverTheCounter.code(), "TWO");
        assert_eq!(YahooClassExchange::OverTheCounter.label(), "上櫃");
        assert_eq!(YahooClassExchange::Emerging.code(), "OES");
        assert_eq!(YahooClassExchange::Emerging.label(), "興櫃");
    }

    /// 驗證三大市場預先整理好的類股數量沒有意外變動。
    #[test]
    fn test_class_category_counts() {
        assert_eq!(LISTED_CLASS_CATEGORIES.len(), 42);
        assert_eq!(OTC_CLASS_CATEGORIES.len(), 35);
        assert_eq!(EMERGING_CLASS_CATEGORIES.len(), 27);
    }

    /// 驗證幾個代表性 `sector_id` 仍對應到預期的類股名稱。
    #[test]
    fn test_known_category_sector_ids() {
        assert_eq!(
            find_category(YahooClassExchange::Listed, 1).map(|category| category.name),
            Some("水泥")
        );
        assert_eq!(
            find_category(YahooClassExchange::OverTheCounter, 122).map(|category| category.name),
            Some("食品")
        );
        assert_eq!(
            find_category(YahooClassExchange::Emerging, 99327).map(|category| category.name),
            Some("文化創意")
        );
    }
}
