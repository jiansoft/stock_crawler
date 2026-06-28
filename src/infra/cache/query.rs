use crate::{
    domain::market_index::MarketIndex,
    infra::database::table::{last_daily_quotes, revenue, stock_exchange_market},
};

use super::share::Share;

impl Share {
    /// 更新目前對外 IP 到快取。
    pub fn set_current_ip(&self, ip: String) {
        if let Ok(mut current_ip) = self.current_ip.write() {
            *current_ip = ip;
        }
    }

    /// 從快取取得目前對外 IP。
    ///
    /// 回傳 `Some(String)` 表示成功（值可能是空字串），`None` 表示讀鎖失敗。
    pub fn get_current_ip(&self) -> Option<String> {
        match self.current_ip.read() {
            Ok(ip) => Some(ip.clone()),
            Err(_) => None,
        }
    }

    /// 寫入或覆蓋單筆台股指數快取。
    ///
    /// 若寫入鎖失敗，回傳原輸入值，讓呼叫端可自行決定是否重試。
    pub async fn set_stock_index(&self, key: String, index: MarketIndex) -> Option<MarketIndex> {
        match self.indices.write() {
            Ok(mut indices) => indices.insert(key, index),
            Err(_) => Some(index),
        }
    }

    /// 依鍵值讀取台股指數快取。
    ///
    /// 未命中或讀鎖失敗時回傳 `None`。
    pub fn get_stock_index(&self, key: &str) -> Option<MarketIndex> {
        match self.indices.read() {
            Ok(cache) => cache.get(key).cloned(),
            Err(_) => None,
        }
    }

    /// 依交易市場代碼取得市場描述資料。
    pub fn get_exchange_market(
        &self,
        id: i32,
    ) -> Option<stock_exchange_market::StockExchangeMarket> {
        self.exchange_markets.get(&id).cloned()
    }

    /// 透過產業名稱取得對應的產業代碼。
    ///
    /// 未命中時回傳 `Some(99)`（未分類）。
    pub fn get_industry_id(&self, name: &str) -> Option<i32> {
        match self.industries.get(name) {
            None => Some(99),
            Some(industry) => Some(*industry),
        }
    }

    /// 透過產業代碼反查第一個符合的產業名稱。
    pub fn get_industry_name(&self, id: i32) -> Option<String> {
        self.industries.iter().find_map(|(key, &value)| {
            if value == id {
                Some(key.to_string())
            } else {
                None
            }
        })
    }

    /// 依股票代號讀取股票主檔快取。
    pub async fn get_stock(&self, symbol: &str) -> Option<crate::domain::registry::entity::Stock> {
        match self.stocks.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }

    /// 判斷股票主檔快取是否包含指定股票代號。
    pub fn stock_contains_key(&self, symbol: &str) -> bool {
        match self.stocks.read() {
            Ok(cache) => cache.contains_key(symbol),
            Err(_) => false,
        }
    }

    /// 取得某檔股票的「最後交易日報價」快取資料。
    pub async fn get_stock_last_price(
        &self,
        symbol: &str,
    ) -> Option<last_daily_quotes::LastDailyQuotes> {
        match self.last_trading_day_quotes.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }

    /// 將單筆月營收資料寫入 `last_revenues` 快取。
    ///
    /// 同月同股號若已有資料，保留原值，不覆蓋舊值。
    pub fn set_last_revenues(&self, revenue: revenue::Revenue) {
        if let Ok(mut last_revenues) = self.last_revenues.write() {
            last_revenues
                .entry(revenue.date)
                .or_insert_with(std::collections::HashMap::new)
                .entry(revenue.stock_symbol.to_string())
                .or_insert(revenue);
        }
    }

    /// 檢查 `last_revenues` 是否存在指定月份與股票代號的資料。
    pub fn last_revenues_contains_key(&self, key1: i64, key2: &str) -> bool {
        self.last_revenues
            .read()
            .map(|cache| {
                cache
                    .get(&key1)
                    .is_some_and(|last_revenue| last_revenue.contains_key(key2))
            })
            .unwrap_or(false)
    }

    /// 更新最後交易日報價快取中的既有股票收盤價。
    ///
    /// 僅更新已存在於快取中的股票（date + closing_price），不新增資料。
    pub async fn set_stock_last_price(
        &self,
        daily_quote: &crate::domain::quote::entity::DailyQuote,
    ) {
        if let Ok(mut last_trading_day_quotes) = self.last_trading_day_quotes.write()
            && let Some(quote) = last_trading_day_quotes.get_mut(&daily_quote.stock_symbol)
        {
            quote.date = daily_quote.date;
            quote.closing_price = daily_quote.closing_price;
        }
    }

    /// 取得最後交易日報價快取資料（與 [`Self::get_stock_last_price`] 等價）。
    pub async fn get_last_trading_day_quotes(
        &self,
        symbol: &str,
    ) -> Option<last_daily_quotes::LastDailyQuotes> {
        match self.last_trading_day_quotes.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::database::table::revenue;

    use super::super::share::Share;

    fn make_test_revenue(stock_symbol: &str, date: i64) -> revenue::Revenue {
        let mut r = revenue::Revenue::new();
        r.stock_symbol = stock_symbol.to_string();
        r.date = date;
        r
    }

    #[test]
    fn test_set_last_revenues_creates_new_month_bucket() {
        let share = Share::new();
        share.set_last_revenues(make_test_revenue("2330", 202501));
        assert!(share.last_revenues_contains_key(202501, "2330"));
    }

    #[test]
    fn static_lookup_tables_have_known_defaults_and_fallbacks() {
        let share = Share::new();
        let listed = share.get_exchange_market(2).unwrap();
        assert_eq!(listed.stock_exchange_market_id, 2);
        assert!(share.get_exchange_market(999).is_none());
        assert_eq!(share.get_industry_id("水泥工業"), Some(1));
        assert_eq!(share.get_industry_id("不存在產業"), Some(99));
        assert_eq!(share.get_industry_name(99), Some("未分類".to_string()));
        assert_eq!(share.get_industry_name(100), None);
    }

    #[test]
    fn current_ip_round_trips_without_loading_external_sources() {
        let share = Share::new();
        assert_eq!(share.get_current_ip(), Some(String::new()));
        share.set_current_ip("203.0.113.1".to_string());
        assert_eq!(share.get_current_ip(), Some("203.0.113.1".to_string()));
    }

    #[tokio::test]
    async fn test_set_and_get_stock_index_round_trips() {
        use crate::domain::market_index::MarketIndex;
        use chrono::NaiveDate;
        use rust_decimal::Decimal;

        let share = Share::new();
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let index = MarketIndex::new(
            "TAIEX".to_string(),
            date,
            Decimal::from(20000),
            Decimal::from(100),
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        );

        let key = "2025-06-01-TAIEX".to_string();
        assert!(share.get_stock_index(&key).is_none());

        share.set_stock_index(key.clone(), index).await;

        let got = share.get_stock_index(&key).unwrap();
        assert_eq!(got.category, "TAIEX");
        assert_eq!(got.date, date);
        assert_eq!(got.index, Decimal::from(20000));
    }
}
