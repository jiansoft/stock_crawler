use std::collections::HashMap;

use chrono::Utc;
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
        // 所有批次寫入在同一入口蓋章，避免各採集器自行決定資料時間。
        let now = Utc::now();
        for snapshot in snapshots.values_mut() {
            snapshot.updated_at = now;
        }
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

    /// 一次取出目前快取內的所有即時報價快照。
    ///
    /// 供「全市場排行」這類需要掃描整份快取的唯讀使用情境（例如 Data API 的
    /// `/api/v1/market/movers`）呼叫。
    ///
    /// # 為什麼是「複製出來」而不是回傳參考或在鎖內排序？
    ///
    /// `stock_snapshots` 是一份 `RwLock` 保護的共用快取，盤中會被採集任務
    /// 高頻寫入（HiStock 與 Yahoo 兩條背景任務）。如果呼叫端在持有讀鎖的
    /// 期間做排序、過濾等運算，寫入端就必須等這些運算做完才能更新報價，
    /// 等於用「即時性」換「少複製一次」，划不來。全市場約兩千筆
    /// [`RealtimeSnapshot`] 的複製成本遠低於延遲即時報價寫入的代價，因此
    /// 這裡選擇在鎖內只做複製、離開鎖之後才讓呼叫端自由運算。
    ///
    /// # 回傳
    ///
    /// - 快取內全部快照的複本；順序不保證（來源是 `HashMap`），呼叫端必須
    ///   自行排序。
    /// - 非交易時段（快取已被 [`Share::clear_stock_snapshots`] 清空）或讀鎖
    ///   毒化時回傳空 `Vec`。
    pub fn all_stock_snapshots(&self) -> Vec<RealtimeSnapshot> {
        self.stock_snapshots
            .read()
            .map(|cache| cache.values().cloned().collect())
            .unwrap_or_default()
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

    /// `all_stock_snapshots` 必須把快取內每一筆都完整複製出來（供全市場排行
    /// 掃描），且在快取為空（非交易時段）時回傳空 `Vec` 而非 panic。
    #[test]
    fn all_stock_snapshots_returns_every_cached_entry() {
        let share = Share::new();
        // 空快取代表非交易時段：必須是空 Vec，呼叫端才能據此切換資料來源。
        assert!(share.all_stock_snapshots().is_empty());

        let mut snapshots = HashMap::new();
        for symbol in ["2330", "2317", "6446"] {
            let mut snapshot = RealtimeSnapshot::new(symbol.to_string(), Decimal::new(100, 0));
            // last_close 與 price 相同可避開 `is_valid_price` 的漲跌幅過濾，
            // 讓這個測試只驗證「複製是否完整」這件事。
            snapshot.last_close = Decimal::new(100, 0);
            snapshot.volume = Decimal::new(1234, 0);
            snapshots.insert(symbol.to_string(), snapshot);
        }
        share.set_stock_snapshots(snapshots);

        let mut symbols: Vec<String> = share
            .all_stock_snapshots()
            .into_iter()
            .map(|snapshot| snapshot.symbol)
            .collect();
        // HashMap 不保證順序，排序後再比對，避免測試因雜湊順序偶發失敗。
        symbols.sort();
        assert_eq!(symbols, vec!["2317", "2330", "6446"]);
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
