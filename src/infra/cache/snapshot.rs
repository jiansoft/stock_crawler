use std::collections::HashMap;

use chrono::Utc;
use rust_decimal::Decimal;

use super::realtime::RealtimeSnapshot;
use super::share::Share;

impl Share {
    /// 取得資料庫最後交易日的收盤價；快取沒有或非正數時回傳 `None`。
    fn get_db_last_close(&self, symbol: &str) -> Option<Decimal> {
        self.last_trading_day_quotes
            .read()
            .ok()
            .and_then(|cache| cache.get(symbol).map(|q| q.closing_price))
            .filter(|&p| p > Decimal::ZERO)
    }

    /// 檢查採集到的股價是否合法（與比對基準相差是否在 10.5% 以內）。
    ///
    /// 比對基準有兩個，任一個通過即視為有效：
    /// - 資料庫最後交易日收盤價。
    /// - 採集站點提供的昨收／參考價（`snapshot_last_close`）。
    ///
    /// 除權息當日的漲跌幅是以「除權息參考價」計算，而資料庫收盤價是除權息前的價格；
    /// 只比對資料庫收盤價會把當天的正常成交價全部當成異常（例如 2542 除息 4 元，
    /// 參考價 41.45、成交 39.05，相對除息前收盤 45.45 跌了 14%）。Yahoo 等站點
    /// 在除權息日回報的昨收即為參考價，因此兩個基準都要納入。
    ///
    /// `price <= 0`（尚未成交）一律回傳 `false`；若兩個基準都沒有有效值，無法比對，視為有效。
    pub fn is_valid_price(
        &self,
        symbol: &str,
        price: Decimal,
        snapshot_last_close: Decimal,
    ) -> bool {
        if price <= Decimal::ZERO {
            return false;
        }

        let site_last_close = Some(snapshot_last_close).filter(|&p| p > Decimal::ZERO);
        let mut baselines = [self.get_db_last_close(symbol), site_last_close]
            .into_iter()
            .flatten()
            .peekable();

        if baselines.peek().is_none() {
            // 如果沒有有效的昨收價，無法進行比較，暫且視為有效
            return true;
        }

        // 10.5% (0.105) 昨收價差做為異常閾值（台股漲跌幅上限 10%）
        // 使用乘法比對比除法運算更安全、且能避免 Decimal 除法時可能產生的精度截斷
        baselines.any(|last_close| (price - last_close).abs() <= last_close * Decimal::new(105, 3))
    }

    /// 以新抓到的完整快照覆蓋快照快取，自動過濾與昨收價相差 10.5% 以上的異常價格，並保留舊有合法值。
    pub fn set_stock_snapshots(&self, mut snapshots: HashMap<String, RealtimeSnapshot>) {
        // 所有批次寫入在同一入口蓋章，避免各採集器自行決定資料時間。
        let now = Utc::now();
        for snapshot in snapshots.values_mut() {
            snapshot.updated_at = now;
        }
        if let Ok(mut cache) = self.stock_snapshots.write() {
            // 檢查每一檔股票的新報價是否異常，若是，則將其價格標記為 0 準備過濾/恢復
            for (symbol, new_snap) in &mut snapshots {
                if !self.is_valid_price(symbol, new_snap.price, new_snap.last_close) {
                    // 價格 0 是尚未成交，不是異常，只過濾不記錄，避免開盤前後洗版
                    if new_snap.price > Decimal::ZERO {
                        tracing::warn!(
                            "過濾異常價格！股票: {}, 採集價格: {}, 昨收價: {}, 站點: {}",
                            symbol,
                            new_snap.price,
                            new_snap.last_close,
                            new_snap.source_site
                        );
                    }
                    new_snap.price = Decimal::ZERO;
                }
            }

            // 如果新報價異常且原本快取中有舊資料，則從舊快取還原，避免直接抹除該股票
            for (symbol, old_snap) in cache.iter() {
                if let Some(new_snap) = snapshots.get_mut(symbol)
                    && new_snap.price == Decimal::ZERO
                {
                    *new_snap = old_snap.clone();
                }
            }

            // 移除新快照中價格依然為 0 的無效資料
            snapshots.retain(|_, snap| snap.price > Decimal::ZERO);

            *cache = snapshots;
        }
    }

    /// 寫入或更新單筆股票報價快照中的最新成交價。
    ///
    /// 若快取內已存在該股票，僅更新 `price`，保留其他欄位。
    /// 若快取內尚無，建立最小快照。
    pub fn set_stock_snapshot_price(&self, symbol: String, price: Decimal) {
        if let Ok(mut cache) = self.stock_snapshots.write() {
            let last_close = cache
                .get(&symbol)
                .map(|s| s.last_close)
                .unwrap_or(Decimal::ZERO);
            if !self.is_valid_price(&symbol, price, last_close) {
                tracing::warn!(
                    "過濾異常價格！股票: {}, 採集價格: {}, 昨收價: {}",
                    symbol,
                    price,
                    last_close
                );
                return;
            }
            if let Some(snapshot) = cache.get_mut(&symbol) {
                snapshot.price = price;
                snapshot.updated_at = Utc::now();
            } else {
                cache.insert(symbol.clone(), RealtimeSnapshot::new(symbol, price));
            }
        }
    }

    /// 寫入或更新單筆股票報價快照中的最新成交價與來源站點。
    pub fn set_stock_snapshot_price_with_source(
        &self,
        symbol: String,
        price: Decimal,
        source_site: impl Into<String>,
    ) {
        let source_site = source_site.into();

        if let Ok(mut cache) = self.stock_snapshots.write() {
            let last_close = cache
                .get(&symbol)
                .map(|s| s.last_close)
                .unwrap_or(Decimal::ZERO);
            if !self.is_valid_price(&symbol, price, last_close) {
                tracing::warn!(
                    "過濾異常價格！股票: {}, 採集價格: {}, 昨收價: {}, 站點: {}",
                    symbol,
                    price,
                    last_close,
                    source_site
                );
                return;
            }
            if let Some(snapshot) = cache.get_mut(&symbol) {
                snapshot.price = price;
                snapshot.source_site = source_site;
                snapshot.updated_at = Utc::now();
            } else {
                let mut snapshot = RealtimeSnapshot::new(symbol.clone(), price);
                snapshot.source_site = source_site;
                cache.insert(symbol, snapshot);
            }
        }
    }

    /// 從快取取得股票報價快照。
    pub fn get_stock_snapshot(&self, symbol: &str) -> Option<RealtimeSnapshot> {
        self.stock_snapshots
            .read()
            .ok()
            .and_then(|cache| cache.get(symbol).cloned())
    }

    /// 回傳快取是否為空；收盤清空後以此辨別非交易時段。
    pub fn stock_snapshots_are_empty(&self) -> bool {
        self.stock_snapshots
            .read()
            .map(|cache| cache.is_empty())
            .unwrap_or(true)
    }

    /// 清空股票報價快照快取。
    pub fn clear_stock_snapshots(&self) {
        if let Ok(mut cache) = self.stock_snapshots.write() {
            *cache = HashMap::new();
        }
    }

    /// 清空昨日收盤快取，避免並發測試透過 `is_valid_price` 互相干擾。
    #[cfg(test)]
    pub fn clear_last_trading_day_quotes(&self) {
        if let Ok(mut q) = self.last_trading_day_quotes.write() {
            q.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::super::realtime::RealtimeSnapshot;
    use super::super::share::Share;

    #[test]
    fn test_set_stock_snapshot_price_preserves_existing_fields() {
        let share = Share::new();
        let mut snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(998, 0));
        snapshot.name = "台積電".to_string();
        snapshot.source_site = "HiStock".to_string();
        snapshot.change = Decimal::new(5, 0);

        let mut snapshots = HashMap::new();
        snapshots.insert("2330".to_string(), snapshot);
        share.set_stock_snapshots(snapshots);

        share.set_stock_snapshot_price("2330".to_string(), Decimal::new(1000, 0));

        let updated = share.get_stock_snapshot("2330").unwrap();
        assert_eq!(updated.price, Decimal::new(1000, 0));
        assert_eq!(updated.name, "台積電");
        assert_eq!(updated.source_site, "HiStock");
        assert_eq!(updated.change, Decimal::new(5, 0));
    }

    #[test]
    fn test_set_stock_snapshot_price_with_source_updates_source_site() {
        let share = Share::new();
        let mut snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(998, 0));
        snapshot.source_site = "Yahoo".to_string();

        let mut snapshots = HashMap::new();
        snapshots.insert("2330".to_string(), snapshot);
        share.set_stock_snapshots(snapshots);

        share.set_stock_snapshot_price_with_source(
            "2330".to_string(),
            Decimal::new(1000, 0),
            "Fugle",
        );

        let updated = share.get_stock_snapshot("2330").unwrap();
        assert_eq!(updated.price, Decimal::new(1000, 0));
        assert_eq!(updated.source_site, "Fugle");
    }

    #[test]
    fn set_stock_snapshots_filters_invalid_new_prices_and_keeps_old_valid_snapshot() {
        let share = Share::new();
        let mut old_snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(100, 0));
        old_snapshot.last_close = Decimal::new(100, 0);
        old_snapshot.source_site = "old".to_string();
        let mut old_map = HashMap::new();
        old_map.insert("2330".to_string(), old_snapshot.clone());
        share.set_stock_snapshots(old_map);

        let mut invalid_update = RealtimeSnapshot::new("2330".to_string(), Decimal::new(200, 0));
        invalid_update.last_close = Decimal::new(100, 0);
        invalid_update.source_site = "new".to_string();
        let mut invalid_map = HashMap::new();
        invalid_map.insert("2330".to_string(), invalid_update);
        invalid_map.insert(
            "2317".to_string(),
            RealtimeSnapshot::new("2317".to_string(), Decimal::ZERO),
        );

        share.set_stock_snapshots(invalid_map);

        let kept = share.get_stock_snapshot("2330").unwrap();
        assert_eq!(kept.price, old_snapshot.price);
        assert_eq!(kept.source_site, "old");
        assert_eq!(share.get_stock_snapshot("2317"), None);
    }

    /// 在測試用 `Share` 的最後交易日報價快取寫入單筆收盤價。
    fn seed_db_last_close(share: &Share, symbol: &str, closing_price: Decimal) {
        use crate::infra::database::table::last_daily_quotes::LastDailyQuotes;

        let mut quote = LastDailyQuotes::new();
        quote.stock_symbol = symbol.to_string();
        quote.closing_price = closing_price;
        share
            .last_trading_day_quotes
            .write()
            .unwrap()
            .insert(symbol.to_string(), quote);
    }

    /// 除息日：資料庫收盤價是除息前價格，站點昨收是除息參考價，成交價只貼近參考價也要放行。
    #[test]
    fn is_valid_price_accepts_price_near_site_reference_on_ex_dividend_day() {
        let share = Share::new();
        // 2542 興富發 2026-09-23 除息 4 元：除息前收盤 45.45、參考價 41.45
        seed_db_last_close(&share, "2542", dec!(45.45));

        assert!(share.is_valid_price("2542", dec!(39.05), dec!(41.45)));
        // 站點仍回報除息前收盤時，無法得知參考價，維持過濾
        assert!(!share.is_valid_price("2542", dec!(39.05), dec!(45.45)));
    }

    /// 任一基準通過即有效，但兩個基準都差太多時仍要過濾。
    #[test]
    fn is_valid_price_rejects_price_far_from_both_baselines() {
        let share = Share::new();
        seed_db_last_close(&share, "6456", dec!(73.6));

        assert!(share.is_valid_price("6456", dec!(78.6), dec!(73.6)));
        assert!(!share.is_valid_price("6456", dec!(188), dec!(73.6)));
        assert!(!share.is_valid_price("6456", Decimal::ZERO, dec!(73.6)));
    }

    /// 沒有任何基準時無法比對，視為有效；只有站點基準時以站點基準比對。
    #[test]
    fn is_valid_price_falls_back_to_site_baseline_when_db_missing() {
        let share = Share::new();

        assert!(share.is_valid_price("2330", dec!(1000), Decimal::ZERO));
        assert!(share.is_valid_price("2330", dec!(1000), dec!(990)));
        assert!(!share.is_valid_price("2330", dec!(1200), dec!(990)));
    }

    #[test]
    fn set_stock_snapshot_price_rejects_outliers_and_accepts_valid_updates() {
        let share = Share::new();
        let mut snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(100, 0));
        snapshot.last_close = Decimal::new(100, 0);
        let mut snapshots = HashMap::new();
        snapshots.insert("2330".to_string(), snapshot);
        share.set_stock_snapshots(snapshots);

        share.set_stock_snapshot_price("2330".to_string(), Decimal::new(200, 0));
        assert_eq!(
            share.get_stock_snapshot("2330").unwrap().price,
            Decimal::new(100, 0)
        );

        share.set_stock_snapshot_price_with_source(
            "2330".to_string(),
            Decimal::new(105, 0),
            "Yahoo",
        );
        let updated = share.get_stock_snapshot("2330").unwrap();
        assert_eq!(updated.price, Decimal::new(105, 0));
        assert_eq!(updated.source_site, "Yahoo");
    }
}
