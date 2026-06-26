use std::collections::HashMap;

use rust_decimal::Decimal;

use super::realtime::RealtimeSnapshot;
use super::share::Share;

impl Share {
    /// 取得最後交易日的收盤價，優先從快取中取得，否則退回使用傳入的備援值。
    fn get_last_close(&self, symbol: &str, fallback: Decimal) -> Decimal {
        self.last_trading_day_quotes
            .read()
            .ok()
            .and_then(|cache| cache.get(symbol).map(|q| q.closing_price))
            .filter(|&p| p > Decimal::ZERO)
            .unwrap_or(fallback)
    }

    /// 檢查採集到的股價是否合法（與上一個交易日的最後收盤價相比，差距是否在 10.5% 以內）。
    ///
    /// 若無昨收價可比對，視為有效並回傳 `true`。
    pub fn is_valid_price(
        &self,
        symbol: &str,
        price: Decimal,
        snapshot_last_close: Decimal,
    ) -> bool {
        if price <= Decimal::ZERO {
            return false;
        }

        let last_close = self.get_last_close(symbol, snapshot_last_close);

        if last_close <= Decimal::ZERO {
            // 如果沒有有效的昨收價，無法進行比較，暫且視為有效
            return true;
        }

        // 10.5% (0.105) 昨收價差做為異常閾值（台股漲跌幅上限 10%）
        // 使用乘法比對比除法運算更安全、且能避免 Decimal 除法時可能產生的精度截斷
        let diff = (price - last_close).abs();
        let limit = last_close * Decimal::new(105, 3);
        diff <= limit
    }

    /// 以新抓到的完整快照覆蓋快照快取，自動過濾與昨收價相差 10.5% 以上的異常價格，並保留舊有合法值。
    pub fn set_stock_snapshots(&self, mut snapshots: HashMap<String, RealtimeSnapshot>) {
        if let Ok(mut cache) = self.stock_snapshots.write() {
            // 檢查每一檔股票的新報價是否異常，若是，則將其價格標記為 0 準備過濾/恢復
            for (symbol, new_snap) in &mut snapshots {
                if !self.is_valid_price(symbol, new_snap.price, new_snap.last_close) {
                    tracing::warn!(
                        "過濾異常價格！股票: {}, 採集價格: {}, 昨收價: {}, 站點: {}",
                        symbol,
                        new_snap.price,
                        new_snap.last_close,
                        new_snap.source_site
                    );
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

    /// 清空股票報價快照快取。
    pub fn clear_stock_snapshots(&self) {
        if let Ok(mut cache) = self.stock_snapshots.write() {
            *cache = HashMap::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rust_decimal::Decimal;

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
