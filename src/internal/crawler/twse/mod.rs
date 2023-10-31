use reqwest::header::HeaderMap;

use crate::util::http;

/// 國際證券辨識
pub mod international_securities_identification_number;
/// 公開申購公告-抽籤日程表
pub mod public;
/// 外資及陸資投資持股
pub mod qualified_foreign_institutional_investor;
/// 台股收盤報價-上市
pub mod quote;
/// 月營收
pub mod revenue;
/// 終止上市公司
pub mod suspend_listing;
/// 台股加權指數
pub mod taiwan_capitalization_weighted_stock_index;

const HOST: &str = "twse.com.tw";

pub(super) async fn build_headers() -> HeaderMap {
    let mut h = HeaderMap::with_capacity(4);
    h.insert("Host", "www.twse.com.tw".parse().unwrap());
    h.insert(
        "Referer",
        "https://www.twse.com.tw/zh/page/trading/exchange/MI_INDEX.html"
            .parse()
            .unwrap(),
    );
    h.insert("X-Requested-With", "XMLHttpRequest".parse().unwrap());
    h.insert(
        "User-Agent",
        http::user_agent::gen_random_ua().parse().unwrap(),
    );
    h
}
