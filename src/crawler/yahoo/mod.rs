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
///
/// 這份結構同時扮演兩個角色：
/// 1. 提供 Yahoo 類股 `sectorId` 的靜態字典。
/// 2. 宣告該類股是否要納入盤中的背景採集任務。
///
/// 因此某些分類即使 `collect_enabled = false`，仍然會保留在常數表中，
/// 方便其他模組查 `sectorId`、顯示名稱或做後續人工比對。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YahooClassCategory {
    /// 類股所屬交易所。
    pub exchange: YahooClassExchange,
    /// Yahoo `class-quote` 頁面的 `sectorId`。
    pub sector_id: u32,
    /// 類股中文名稱。
    pub name: &'static str,
    /// 是否納入盤中 Yahoo 類股採集任務。
    ///
    /// `false` 代表該分類只保留在靜態字典中，不會被
    /// [`crate::crawler::yahoo::price::class_quote::all_class_categories`]
    /// 納入背景輪詢清單。
    pub collect_enabled: bool,
}

impl YahooClassCategory {
    /// 建立會納入盤中採集的 Yahoo 類股分類。
    ///
    /// 適用於一般產業股或其他需要持續刷新共享快取的分類。
    pub const fn enabled(exchange: YahooClassExchange, sector_id: u32, name: &'static str) -> Self {
        Self {
            exchange,
            sector_id,
            name,
            collect_enabled: true,
        }
    }

    /// 建立只保留在類股字典、但不納入盤中採集的 Yahoo 類股分類。
    ///
    /// 目前主要用在認購、認售、指數類等不希望進入盤中輪詢的分類。
    pub const fn disabled(
        exchange: YahooClassExchange,
        sector_id: u32,
        name: &'static str,
    ) -> Self {
        Self {
            exchange,
            sector_id,
            name,
            collect_enabled: false,
        }
    }
}

/// Yahoo 上市類股分類。
///
/// 這份清單會保留完整 Yahoo 字典，即使部分分類已標成不採集。
pub const LISTED_CLASS_CATEGORIES: &[YahooClassCategory] = &[
    YahooClassCategory::enabled(YahooClassExchange::Listed, 1, "水泥"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 2, "食品"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 3, "塑膠"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 4, "紡織"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 6, "電機機械"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 7, "電器電纜"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 9, "玻璃"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 10, "造紙"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 11, "鋼鐵"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 12, "橡膠"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 13, "汽車"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 19, "營建"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 20, "航運"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 21, "觀光餐旅"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 22, "金融業"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 24, "貿易百貨"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 25, "存託憑證"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 26, "ETF"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 29, "受益證券"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 30, "其他"),
    YahooClassCategory::disabled(YahooClassExchange::Listed, 31, "市認購"),
    YahooClassCategory::disabled(YahooClassExchange::Listed, 32, "市認售"),
    YahooClassCategory::disabled(YahooClassExchange::Listed, 33, "指數類"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 37, "化學"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 38, "生技"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 39, "油電燃氣"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 40, "半導體"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 41, "電腦週邊"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 42, "光電"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 43, "通訊網路"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 44, "電子零組件"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 45, "電子通路"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 46, "資訊服務"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 47, "其他電子"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 48, "ETN"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 49, "創新板"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 51, "市牛證"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 52, "市熊證"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 93, "綠能環保"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 94, "數位雲端"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 95, "運動休閒"),
    YahooClassCategory::enabled(YahooClassExchange::Listed, 96, "居家生活"),
];

/// Yahoo 上櫃類股分類。
///
/// 這份清單會保留完整 Yahoo 字典，即使部分分類已標成不採集。
pub const OTC_CLASS_CATEGORIES: &[YahooClassCategory] = &[
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 121, "生技"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 122, "食品"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 123, "塑膠"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 124, "紡織"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 125, "電機"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 126, "電器電纜"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 130, "鋼鐵"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 138, "營建"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 139, "航運"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 140, "觀光餐旅"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 141, "金融"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 142, "居家生活"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 145, "其他"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 151, "化學"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 153, "半導體"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 154, "電腦週邊"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 155, "光電"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 156, "通訊網路"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 157, "電子零組件"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 158, "電子通路"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 159, "資訊服務"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 160, "其他電子"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 161, "油電燃氣"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 163, "公司債"),
    YahooClassCategory::disabled(YahooClassExchange::OverTheCounter, 165, "認購"),
    YahooClassCategory::disabled(YahooClassExchange::OverTheCounter, 166, "認售"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 167, "牛證"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 169, "文化創意"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 170, "農業科技業"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 171, "電子商務"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 172, "ETF"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 173, "ETN"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 97, "綠能環保"),
    YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 98, "運動休閒"),
    YahooClassCategory::disabled(YahooClassExchange::OverTheCounter, 33, "指數類"),
];

/// Yahoo 興櫃類股分類。
///
/// 這份清單會保留完整 Yahoo 字典，即使部分分類已標成不採集。
pub const EMERGING_CLASS_CATEGORIES: &[YahooClassCategory] = &[
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99301, "食品"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99303, "紡織"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99304, "電機"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99306, "化學工業"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99307, "生技"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99309, "鋼鐵"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99311, "半導體"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99312, "電腦週邊"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99313, "光電"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99314, "通信網路"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99315, "電子零組件"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99316, "電子通路"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99317, "資訊服務"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99318, "其他電子"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99319, "營建"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99320, "航運"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99321, "觀光餐旅"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99322, "金融"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99323, "居家生活"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99324, "油電燃氣"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99325, "其他"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99327, "文化創意"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99328, "基金黃金"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99329, "農業科技業"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99330, "數位雲端"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99331, "綠能環保"),
    YahooClassCategory::enabled(YahooClassExchange::Emerging, 99332, "運動休閒"),
];

/// 依交易所取得 Yahoo 類股分類。
///
/// 此函式回傳的是完整字典，不會主動過濾 `collect_enabled = false` 的項目。
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

    /// 驗證認購 / 認售 / 指數類等分類會在常數表上直接標成不採集。
    #[test]
    fn test_collect_enabled_marks_disabled_categories() {
        let listed_call = find_category(YahooClassExchange::Listed, 31).unwrap();
        let listed_put = find_category(YahooClassExchange::Listed, 32).unwrap();
        let listed_index = find_category(YahooClassExchange::Listed, 33).unwrap();
        let otc_call = find_category(YahooClassExchange::OverTheCounter, 165).unwrap();
        let otc_put = find_category(YahooClassExchange::OverTheCounter, 166).unwrap();
        let otc_index = find_category(YahooClassExchange::OverTheCounter, 33).unwrap();
        let listed_semiconductor = find_category(YahooClassExchange::Listed, 40).unwrap();

        assert!(!listed_call.collect_enabled);
        assert!(!listed_put.collect_enabled);
        assert!(!listed_index.collect_enabled);
        assert!(!otc_call.collect_enabled);
        assert!(!otc_put.collect_enabled);
        assert!(!otc_index.collect_enabled);
        assert!(listed_semiconductor.collect_enabled);
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
