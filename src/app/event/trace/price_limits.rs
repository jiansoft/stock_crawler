//! # 開盤前載入交易所公告的漲跌停價（H46）
//!
//! 盤中驗價（`Share::is_valid_price`）原本只有 Yahoo 類股報價會帶當日漲跌停價，其餘股票用固定的
//! 10.5% 門檻。國外成分 ETF、債券 ETF 沒有漲跌幅限制，又多半不在 Yahoo 類股裡：2026-11-06 加權
//! 下跌 4.5%，00954 盤中真實低點 12.6（昨收 14.21，−11.3%）被當成異常價格濾掉，同日還有 00735、
//! 00911、00951、00662 等，盤中價格追蹤因此看不到這些價。
//!
//! 開盤前讀上市 TWT84U 與上櫃收盤行情的次日漲跌停價寫入快取：
//! - **無漲跌幅限制**的證券一律採用：這個屬性不會每天變，前一個交易日的資料也適用。
//! - **漲跌停區間**只採用確定是今天的上市資料（TWT84U 資料日期＝今天）；上櫃的次日漲跌停價
//!   要知道「資料日期的下一個交易日是不是今天」才能用，遇到連假或資料晚更新容易套錯日期，
//!   一般股票本來就由固定門檻處理，這裡不冒這個險。
//!
//! Yahoo 類股報價之後每輪仍會覆蓋同一檔的限制，兩者並存時以盤中較新的 Yahoo 為準。

use anyhow::{Result, anyhow};
use chrono::{Local, NaiveDate};

use crate::infra::{
    cache::{PriceLimit, SHARE},
    crawler::{share::ExchangePriceLimits, tpex, twse},
};

/// 一個市場這次寫入快取的筆數。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Applied {
    /// 無漲跌幅限制的筆數。
    unlimited: usize,
    /// 漲跌停區間的筆數。
    ranges: usize,
}

/// 依資料日期挑出可以當成「今天」限制的項目。
///
/// `ranges_apply_today` 為真時連區間一起採用，否則只留無漲跌幅限制。
fn usable_limits(
    data: ExchangePriceLimits,
    ranges_apply_today: bool,
) -> (Vec<(String, PriceLimit)>, Applied) {
    let mut applied = Applied::default();
    let limits = data
        .limits
        .into_iter()
        .filter(|(_, limit)| match limit {
            PriceLimit::Unlimited => {
                applied.unlimited += 1;
                true
            }
            PriceLimit::Range { .. } if ranges_apply_today => {
                applied.ranges += 1;
                true
            }
            PriceLimit::Range { .. } | PriceLimit::Unknown => false,
        })
        .collect();
    (limits, applied)
}

/// 把一個市場的資料寫入今天的漲跌停快取，回傳資料日期與寫入筆數。
fn apply(
    data: ExchangePriceLimits,
    today: NaiveDate,
    ranges_apply_today: bool,
) -> (NaiveDate, Applied) {
    let date = data.date;
    let (limits, applied) = usable_limits(data, ranges_apply_today);
    SHARE.set_price_limits(today, limits);
    (date, applied)
}

/// 讀上市、上櫃的漲跌停價寫入快取；兩個市場互不影響，一邊失敗只記 warn。
///
/// # Errors
///
/// 兩個市場都失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    execute_on(Local::now().date_naive()).await
}

/// [`execute`] 的實作，`today` 由呼叫端指定以便測試。
async fn execute_on(today: NaiveDate) -> Result<()> {
    let (listed, otc) = tokio::join!(twse::price_limit::visit(), tpex::price_limit::visit());

    let listed = listed.map(|data| {
        let ranges_apply_today = data.date == today;
        apply(data, today, ranges_apply_today)
    });
    let otc = otc.map(|data| apply(data, today, false));

    match (&listed, &otc) {
        (Ok((listed_date, listed)), Ok((otc_date, otc))) => tracing::info!(
            "交易所漲跌停價已載入: 上市 資料日={listed_date} 無限制={} 區間={}；上櫃 資料日={otc_date} 無限制={}",
            listed.unlimited,
            listed.ranges,
            otc.unlimited
        ),
        (Ok(_), Err(why)) => tracing::warn!("上櫃次日漲跌停價載入失敗，只用上市資料: {why:#}"),
        (Err(why), Ok(_)) => tracing::warn!("上市漲跌停價載入失敗，只用上櫃資料: {why:#}"),
        (Err(listed), Err(otc)) => {
            return Err(anyhow!(
                "交易所漲跌停價載入失敗：上市 {listed:#}；上櫃 {otc:#}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).expect("valid date")
    }

    fn range(down: i64, up: i64) -> PriceLimit {
        PriceLimit::Range {
            down: Decimal::new(down, 2),
            up: Decimal::new(up, 2),
        }
    }

    /// 資料是今天的才採用區間；不是今天的只留無漲跌幅限制。
    #[test]
    fn ranges_only_when_data_is_for_today() {
        let data = || ExchangePriceLimits {
            date: date(6),
            limits: vec![
                ("00954".to_owned(), PriceLimit::Unlimited),
                ("1718".to_owned(), range(881, 1075)),
            ],
        };

        let (limits, applied) = usable_limits(data(), true);
        assert_eq!(limits.len(), 2);
        assert_eq!(
            applied,
            Applied {
                unlimited: 1,
                ranges: 1
            }
        );

        let (limits, applied) = usable_limits(data(), false);
        assert_eq!(limits, vec![("00954".to_owned(), PriceLimit::Unlimited)]);
        assert_eq!(
            applied,
            Applied {
                unlimited: 1,
                ranges: 0
            }
        );
    }

    fn twse_body(data_date: &str, unlimited: &str, ranged: &str) -> serde_json::Value {
        serde_json::json!({
            "stat": "OK", "date": data_date,
            "fields": ["證券代號","證券名稱","漲停價","開盤競價基準","跌停價"],
            "data": [
                [unlimited, "測試無限制", "9,999.95", "14.21", "0.01"],
                [ranged, "測試區間", "10.75", "9.78", "8.81"]
            ]
        })
    }

    fn tpex_body() -> serde_json::Value {
        serde_json::json!({
            "tables": [{
                "date": "115/11/05",
                "fields": ["代號","名稱","次日漲停價 ","次日跌停價"],
                "data": [
                    ["79985", "測試債券 ETF", "9,999.95", "0.01"],
                    ["79986", "測試上櫃區間", "563.00", "461.00"]
                ]
            }]
        })
    }

    async fn mount(server: &MockServer, route: &str, status: u16, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(server)
            .await;
    }

    /// 上市資料是今天的：無限制與區間都寫入；上櫃只寫無限制（次日區間不採用）。
    /// 用 1991 年的日期，不會蓋到今天盤中真正在用的限制。
    #[tokio::test]
    async fn execute_writes_today_limits_from_both_markets() {
        let today = NaiveDate::from_ymd_opt(1991, 1, 3).expect("valid date");
        let twse = MockServer::start().await;
        let tpex = MockServer::start().await;
        mount(
            &twse,
            "/rwd/zh/variation/TWT84U",
            200,
            twse_body("19910103", "79983", "79984"),
        )
        .await;
        mount(&tpex, "/www/zh-tw/afterTrading/otc", 200, tpex_body()).await;

        with_origins(
            &[
                ("https://www.twse.com.tw", &twse.uri()),
                ("https://www.tpex.org.tw", &tpex.uri()),
            ],
            execute_on(today),
        )
        .await
        .expect("兩個市場都成功");

        assert_eq!(SHARE.price_limit_on("79983", today), PriceLimit::Unlimited);
        assert_eq!(SHARE.price_limit_on("79984", today), range(881, 1075));
        assert_eq!(SHARE.price_limit_on("79985", today), PriceLimit::Unlimited);
        assert_eq!(SHARE.price_limit_on("79986", today), PriceLimit::Unknown);
    }

    /// 上市資料還是前一個交易日的：只寫無限制；上櫃失敗不影響上市。兩邊都失敗才回錯誤。
    #[tokio::test]
    async fn execute_keeps_unlimited_only_for_stale_listed_data() {
        let today = NaiveDate::from_ymd_opt(1991, 1, 4).expect("valid date");
        let twse = MockServer::start().await;
        let tpex = MockServer::start().await;
        mount(
            &twse,
            "/rwd/zh/variation/TWT84U",
            200,
            twse_body("19910103", "79987", "79988"),
        )
        .await;
        mount(
            &tpex,
            "/www/zh-tw/afterTrading/otc",
            404,
            serde_json::json!({}),
        )
        .await;

        let origins = [
            ("https://www.twse.com.tw", twse.uri()),
            ("https://www.tpex.org.tw", tpex.uri()),
        ];
        let origins = origins
            .each_ref()
            .map(|(origin, uri)| (*origin, uri.as_str()));
        with_origins(&origins, execute_on(today))
            .await
            .expect("上市成功就不算失敗");
        assert_eq!(SHARE.price_limit_on("79987", today), PriceLimit::Unlimited);
        assert_eq!(SHARE.price_limit_on("79988", today), PriceLimit::Unknown);

        let down = MockServer::start().await;
        let message = with_origins(
            &[
                ("https://www.twse.com.tw", &down.uri()),
                ("https://www.tpex.org.tw", &down.uri()),
            ],
            execute_on(today),
        )
        .await
        .expect_err("兩個市場都失敗要回錯誤")
        .to_string();
        assert!(message.contains("交易所漲跌停價載入失敗"), "{message}");
    }
}
