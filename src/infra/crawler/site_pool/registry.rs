//! # 站點池清單 (Registry)
//!
//! 此子模組集中定義「最新成交價」與「完整報價」兩條路徑可用的站點清單，
//! 包含統一的函式指標型別、站點描述結構，以及把各站台 `StockInfo`
//! 關聯函式包裝成一致函式指標的巨集。
//!
//! 把清單與輪詢流程分開，是為了讓「要用哪些站台」這件事集中在一個檔案，
//! 調整站點順序或增刪來源時不必動到輪詢與統計邏輯。

use std::{future::Future, pin::Pin};

use anyhow::Result;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;

use crate::{
    core::declare,
    infra::crawler::{
        StockInfo, biggo::BigGo, cmoney::CMoney, cnyes::CnYes, fugle::Fugle, megatime::PcHome,
        nstock::NStock, yahoo::Yahoo,
    },
};

/// 「最新成交價」非同步抓取函式的 boxed future 型別。
///
/// 這個型別別名用來收斂各站點 `async_trait` 產生出的回傳型別，
/// 讓站點池可以用一致的函式指標簽名儲存不同來源。
type StockPriceFuture<'a> = Pin<Box<dyn Future<Output = Result<Decimal>> + Send + 'a>>;
/// 「完整報價」非同步抓取函式的 boxed future 型別。
///
/// 用途與 [`StockPriceFuture`] 相同，只是回傳內容改為 [`declare::StockQuotes`]。
type StockQuotesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<declare::StockQuotes>> + Send + 'a>>;
/// 「最新成交價」站點 wrapper 函式的統一函式指標型別。
type StockPriceFetcher = for<'a> fn(&'a str) -> StockPriceFuture<'a>;
/// 「完整報價」站點 wrapper 函式的統一函式指標型別。
type StockQuotesFetcher = for<'a> fn(&'a str) -> StockQuotesFuture<'a>;

/// 單一「股價」站點的描述。
///
/// 將站點名稱與對應抓價函式綁在一起，避免名稱陣列與函式陣列分離後產生順序錯位。
#[derive(Clone, Copy)]
pub(super) struct PriceSite {
    pub(super) name: &'static str,
    pub(super) fetch: StockPriceFetcher,
}

/// 單一「完整報價」站點的描述。
///
/// 結構與 [`PriceSite`] 相同，但抓取的是開高低收、漲跌幅等完整報價資料。
#[derive(Clone, Copy)]
pub(super) struct QuoteSite {
    pub(super) name: &'static str,
    pub(super) fetch: StockQuotesFetcher,
}

/// 產生「最新成交價」wrapper 函式。
///
/// `async_trait` 產生的關聯函式型別，無法直接穩定地放進模組層級常數陣列；
/// 因此透過此巨集產生一層薄 wrapper，讓站點池可以持有一致的函式指標型別。
///
/// # 使用方式
/// ```ignore
/// define_stock_price_fetcher!(
///     "將 FooSite 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
///     fetch_foosite_price,
///     FooSite
/// );
/// ```
macro_rules! define_stock_price_fetcher {
    ($doc:literal, $fn_name:ident, $site:ty) => {
        #[doc = $doc]
        fn $fn_name<'a>(stock_symbol: &'a str) -> StockPriceFuture<'a> {
            <$site as StockInfo>::get_stock_price(stock_symbol)
        }
    };
}

/// 產生「完整報價」wrapper 函式。
///
/// 用途與 [`define_stock_price_fetcher`] 相同，只是包裝的是
/// `StockInfo::get_stock_quotes`，供完整報價站點池重複使用。
///
/// # 使用方式
/// ```ignore
/// define_stock_quotes_fetcher!(
///     "將 FooSite 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
///     fetch_foosite_quotes,
///     FooSite
/// );
/// ```
macro_rules! define_stock_quotes_fetcher {
    ($doc:literal, $fn_name:ident, $site:ty) => {
        #[doc = $doc]
        fn $fn_name<'a>(stock_symbol: &'a str) -> StockQuotesFuture<'a> {
            <$site as StockInfo>::get_stock_quotes(stock_symbol)
        }
    };
}

define_stock_price_fetcher!(
    "將 Yahoo 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_yahoo_price,
    Yahoo
);
define_stock_price_fetcher!(
    "將 Fugle 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_fugle_price,
    Fugle
);
define_stock_price_fetcher!(
    "將 NStock 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_nstock_price,
    NStock
);
define_stock_price_fetcher!(
    "將 CMoney 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_cmoney_price,
    CMoney
);
define_stock_price_fetcher!(
    "將 CnYes 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_cnyes_price,
    CnYes
);
define_stock_price_fetcher!(
    "將 PcHome 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_pchome_price,
    PcHome
);
define_stock_price_fetcher!(
    "將 BigGo 的 `StockInfo::get_stock_price` 包裝成可放入最新成交價站點池的函式指標。",
    fetch_biggo_price,
    BigGo
);

define_stock_quotes_fetcher!(
    "將 Fugle 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_fugle_quotes,
    Fugle
);
define_stock_quotes_fetcher!(
    "將 NStock 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_nstock_quotes,
    NStock
);
define_stock_quotes_fetcher!(
    "將 CMoney 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_cmoney_quotes,
    CMoney
);
define_stock_quotes_fetcher!(
    "將 CnYes 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_cnyes_quotes,
    CnYes
);
define_stock_quotes_fetcher!(
    "將 PcHome 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_pchome_quotes,
    PcHome
);
define_stock_quotes_fetcher!(
    "將 BigGo 的 `StockInfo::get_stock_quotes` 包裝成可放入完整報價站點池的函式指標。",
    fetch_biggo_quotes,
    BigGo
);

/// 所有可用的「最新成交價」站點池。
///
/// 此順序同時代表 round-robin 的輪詢候選順序。
/// `HiStock` 已從此站點池移除，因為目前即時股價採集改由它自己的背景排程負責。
///
/// 目前站點順序如下：
/// - `Yahoo`
/// - `Fugle`
/// - `NStock`
/// - `CMoney`
/// - `CnYes`
/// - `PcHome`
/// - `BigGo`
///
/// `BigGo` 於 2026-10-05 加入：盤中 09:48～09:58 每 30 秒比對 12 檔（上市、上櫃、ETF）共 220 筆，
/// 95% 落在證交所 MIS 的最佳買賣價之間，其餘 11 筆都只差一檔，且都發生在雙方快照時間相差約
/// 30～60 秒時；無錯誤、無限流。當天未成交的股票 BigGo 回前一個交易日的快照，爬蟲會拒收。
///
/// `Yuanta` 已從此站點池移除，因為其資料目前觀察到為前一交易日資料，
/// 不符合即時追蹤用途。
///
/// `Winvest` 也已移除：2026-09 改版後只剩 `QueryRecentDailyPrice`（盤後日 K，官方註明未取得
/// 盤中資訊授權）。2026-09-30 盤中 09:17／11:03／13:12 三次實測 10 檔，`KlineDatetime` 全是
/// 前一交易日、價格停在昨收；爬蟲的「非今日 K 線」防呆雖會擋下，但每天仍白打上萬次請求。
pub(super) const ALL_PRICE_SITES: [PriceSite; 7] = [
    PriceSite {
        name: "Yahoo",
        fetch: fetch_yahoo_price,
    },
    PriceSite {
        name: "Fugle",
        fetch: fetch_fugle_price,
    },
    PriceSite {
        name: "NStock",
        fetch: fetch_nstock_price,
    },
    PriceSite {
        name: "CMoney",
        fetch: fetch_cmoney_price,
    },
    PriceSite {
        name: "CnYes",
        fetch: fetch_cnyes_price,
    },
    PriceSite {
        name: "PcHome",
        fetch: fetch_pchome_price,
    },
    PriceSite {
        name: "BigGo",
        fetch: fetch_biggo_price,
    },
];

/// 追蹤股票備援採集用的「最新成交價」站點池：[`ALL_PRICE_SITES`] 去掉 Yahoo。
///
/// Yahoo 的 `get_stock_price` 先讀共享即時快取，而備援採集要更新的正是這份快取；
/// 輪到 Yahoo 時只會讀回原值（2026-10-07 正式機 7,846 次、avg 7ms），白白占掉一輪。
pub(super) static BACKUP_PRICE_SITES: Lazy<Vec<PriceSite>> = Lazy::new(|| {
    ALL_PRICE_SITES
        .iter()
        .copied()
        .filter(|site| site.name != "Yahoo")
        .collect()
});

/// 所有可用的「完整報價」站點池。
///
/// 這條路徑只保留目前仍用於單股完整報價備援的站點，
/// `HiStock` 也已改由它自己的背景排程負責，不再納入此站點池。
///
/// 目前站點順序如下：
/// - `Fugle`
/// - `NStock`
/// - `CMoney`
/// - `CnYes`
/// - `PcHome`
/// - `BigGo`（加入原因同 [`ALL_PRICE_SITES`]）
///
/// `Yuanta` 已從此站點池移除，因為其資料目前觀察到為前一交易日資料，
/// 不適合用作即時完整報價來源。`Winvest` 移除原因同 [`ALL_PRICE_SITES`]。
pub(super) const ALL_QUOTE_SITES: [QuoteSite; 6] = [
    QuoteSite {
        name: "Fugle",
        fetch: fetch_fugle_quotes,
    },
    QuoteSite {
        name: "NStock",
        fetch: fetch_nstock_quotes,
    },
    QuoteSite {
        name: "CMoney",
        fetch: fetch_cmoney_quotes,
    },
    QuoteSite {
        name: "CnYes",
        fetch: fetch_cnyes_quotes,
    },
    QuoteSite {
        name: "PcHome",
        fetch: fetch_pchome_quotes,
    },
    QuoteSite {
        name: "BigGo",
        fetch: fetch_biggo_quotes,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 備援站點池只少了 Yahoo，其他站點與順序不變。
    #[test]
    fn backup_price_sites_exclude_only_yahoo() {
        let names = BACKUP_PRICE_SITES
            .iter()
            .map(|site| site.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["Fugle", "NStock", "CMoney", "CnYes", "PcHome", "BigGo"]
        );
    }
}
