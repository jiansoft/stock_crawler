use std::collections::HashMap;

use chrono::{Local, NaiveDate, Utc};
use rust_decimal::Decimal;

use super::realtime::{PriceLimit, RealtimeSnapshot};
use crate::core::declare::StockExchangeMarket;

/// 上市櫃的異常價格閾值：漲跌幅上限 10% 再加 0.5% 容差。
const PRICE_LIMIT_TOLERANCE: Decimal = Decimal::from_parts(105, 0, 0, false, 3);

/// 興櫃的異常價格閾值：興櫃無漲跌幅限制，只擋明顯錯誤的值
/// （例如 HiStock 偶發回報 0.08 而昨收 227.5）。
const EMERGING_TOLERANCE: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
use super::share::Share;

impl Share {
    /// 取得資料庫最後交易日的收盤價；快取沒有或非正數時回傳 `None`。
    /// 股票主檔記載的市場別是否為興櫃；主檔查無此代號時視為否。
    fn is_emerging_stock(&self, symbol: &str) -> bool {
        self.stocks
            .read()
            .ok()
            .and_then(|stocks| {
                stocks
                    .get(symbol)
                    .map(|stock| stock.market_id() == StockExchangeMarket::Emerging.serial())
            })
            .unwrap_or(false)
    }

    /// 取得當日除權息或恢復買賣參考價；當天沒有這類事件時為 `None`。
    fn get_ex_rights_reference_price(&self, symbol: &str) -> Option<Decimal> {
        self.ex_rights_reference_prices
            .read()
            .ok()
            .and_then(|prices| prices.get(symbol).copied())
    }

    /// 以當日除權息與恢復買賣參考價整批覆寫快取（前一個交易日的值會被清掉）。
    pub fn set_ex_rights_reference_prices(&self, prices: HashMap<String, Decimal>) {
        if let Ok(mut cache) = self.ex_rights_reference_prices.write() {
            *cache = prices;
        }
    }

    fn get_db_last_close(&self, symbol: &str) -> Option<Decimal> {
        self.last_trading_day_quotes
            .read()
            .ok()
            .and_then(|cache| cache.get(symbol).map(|q| q.closing_price))
            .filter(|&p| p > Decimal::ZERO)
    }

    /// 檢查採集到的股價是否合法（與比對基準相差是否在 10.5% 以內）。
    ///
    /// 比對基準有三個，任一個通過即視為有效：
    /// - 資料庫最後交易日收盤價。
    /// - 採集站點提供的昨收／參考價（`snapshot_last_close`）。
    /// - 當日除權息或減資／分割恢復買賣參考價（[`Self::set_ex_rights_reference_prices`]
    ///   由資料庫股利事件與 `corporate_action` 算出）。
    ///
    /// 除權息當日的漲跌幅是以「除權息參考價」計算，而資料庫收盤價是除權息前的價格；
    /// 只比對資料庫收盤價會把當天的正常成交價全部當成異常（例如 2542 除息 4 元，
    /// 參考價 41.45、成交 39.05，相對除息前收盤 45.45 跌了 14%）。Yahoo 等站點
    /// 在除權息日回報的昨收即為參考價，但 HiStock 回報的是除權息前收盤
    /// （1235 興泰 2026-09-24 除權息，參考價 38.71，HiStock 仍給 41.15，整天被誤濾），
    /// 因此需要自行計算的參考價作為第三個基準。
    ///
    /// `price <= 0`（尚未成交）一律回傳 `false`；若兩個基準都沒有有效值，無法比對，視為有效。
    ///
    /// 興櫃股票（依股票主檔的市場別判斷）沒有漲跌幅限制，改用 [`EMERGING_TOLERANCE`]，見
    /// [`Self::is_valid_price_for_market`]。
    pub fn is_valid_price(
        &self,
        symbol: &str,
        price: Decimal,
        snapshot_last_close: Decimal,
    ) -> bool {
        let emerging = self.is_emerging_stock(symbol);
        self.is_valid_price_for_market(symbol, price, snapshot_last_close, emerging)
    }

    /// 同 [`Self::is_valid_price`]，但由呼叫端指定是否為興櫃股票。
    ///
    /// 來源本身已知市場別時使用（例如 Yahoo 興櫃類股），可涵蓋尚未寫入股票主檔的新興櫃股。
    /// 興櫃無漲跌幅限制，昨收又是前一日加權平均價，單日偏離 10% 以上很常見
    /// （2026-09-24 有 12 檔興櫃被誤濾，如 7934 昨收 552.86、成交 630）；
    /// 只以 [`EMERGING_TOLERANCE`] 擋明顯錯誤的值。
    ///
    /// 當天有來源提供漲跌幅限制時以它為準（[`Self::set_price_limits`]）：有漲跌停價就照區間判斷，
    /// 沒有限制的（國外成分 ETF）改用 [`EMERGING_TOLERANCE`]。固定的 10.5% 門檻會誤濾
    /// 槓桿 ETF（00631L 的漲跌幅是 ±20%）與沒有漲跌幅限制的 ETF（00715L 2026-10-02 上漲 11.3%）。
    pub fn is_valid_price_for_market(
        &self,
        symbol: &str,
        price: Decimal,
        snapshot_last_close: Decimal,
        emerging: bool,
    ) -> bool {
        if price <= Decimal::ZERO {
            return false;
        }

        let today = Local::now().date_naive();
        let emerging = match self.price_limit_on(symbol, today) {
            PriceLimit::Range { down, up } => return price >= down && price <= up,
            PriceLimit::Unlimited => true,
            PriceLimit::Unknown => emerging,
        };

        let site_last_close = Some(snapshot_last_close).filter(|&p| p > Decimal::ZERO);
        let mut baselines = [
            self.get_db_last_close(symbol),
            site_last_close,
            self.get_ex_rights_reference_price(symbol),
        ]
        .into_iter()
        .flatten()
        .peekable();

        if baselines.peek().is_none() {
            // 如果沒有有效的昨收價，無法進行比較，暫且視為有效
            return true;
        }

        let tolerance = if emerging {
            EMERGING_TOLERANCE
        } else {
            PRICE_LIMIT_TOLERANCE
        };
        // 使用乘法比對比除法運算更安全、且能避免 Decimal 除法時可能產生的精度截斷
        baselines.any(|last_close| (price - last_close).abs() <= last_close * tolerance)
    }

    /// 記錄各股票當天的漲跌幅限制（`date` 為台北時間的取得日期）；`Unknown` 不記錄。
    pub fn set_price_limits(
        &self,
        date: NaiveDate,
        limits: impl IntoIterator<Item = (String, PriceLimit)>,
    ) {
        if let Ok(mut cache) = self.price_limits.write() {
            for (symbol, limit) in limits {
                if limit != PriceLimit::Unknown {
                    cache.insert(symbol, (date, limit));
                }
            }
        }
    }

    /// 取得指定日期的漲跌幅限制；沒有記錄或不是當天的記錄回傳 `Unknown`。
    pub fn price_limit_on(&self, symbol: &str, date: NaiveDate) -> PriceLimit {
        self.price_limits
            .read()
            .ok()
            .and_then(|cache| cache.get(symbol).copied())
            .filter(|(limit_date, _)| *limit_date == date)
            .map(|(_, limit)| limit)
            .unwrap_or_default()
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

    /// 1235 興泰 2026-09-24 除權息：資料庫與 HiStock 都給除權息前收盤 41.15，
    /// 只有自行計算的參考價 38.71 能讓當天 35.5～40.2 的正常成交通過。
    #[test]
    fn is_valid_price_accepts_price_near_computed_ex_rights_reference() {
        let share = Share::new();
        seed_db_last_close(&share, "1235", dec!(41.15));
        assert!(!share.is_valid_price("1235", dec!(35.8), dec!(41.15)));

        share.set_ex_rights_reference_prices(HashMap::from([("1235".to_string(), dec!(38.71))]));
        assert!(share.is_valid_price("1235", dec!(35.8), dec!(41.15)));
        // 參考價的 ±10.5% 之外仍要過濾
        assert!(!share.is_valid_price("1235", dec!(30), dec!(41.15)));

        // 下一個交易日整批覆寫後，前一天的參考價不再生效
        share.set_ex_rights_reference_prices(HashMap::new());
        assert!(!share.is_valid_price("1235", dec!(35.8), dec!(41.15)));
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

    /// 興櫃沒有漲跌幅限制：主檔標為興櫃時放寬到 ±50%，但仍擋明顯錯誤的值。
    #[test]
    fn is_valid_price_relaxes_tolerance_for_emerging_stocks() {
        use crate::domain::registry::entity::Stock;

        let share = Share::new();
        // 4925 智微：上櫃規則下 +14% 會被濾掉
        assert!(!share.is_valid_price("4925", dec!(133.5), dec!(117.09)));

        share.stocks.write().unwrap().insert(
            "4925".to_string(),
            Stock::register("4925".to_string(), "智微".to_string(), 5, 1),
        );
        assert!(share.is_valid_price("4925", dec!(133.5), dec!(117.09)));
        assert!(!share.is_valid_price("4925", dec!(1.2), dec!(117.09)));
    }

    /// 當天有漲跌停價時照區間判斷：槓桿 ETF 的 ±20% 不被固定 10.5% 誤濾，超出區間仍擋下。
    #[test]
    fn todays_price_limit_range_overrides_the_fixed_tolerance() {
        use super::super::realtime::PriceLimit;

        let share = Share::new();
        let today = chrono::Local::now().date_naive();
        // 00631L：昨收 39.62，漲跌停 31.70～47.54。
        assert!(!share.is_valid_price("00631L", dec!(45), dec!(39.62)));
        share.set_price_limits(
            today,
            [(
                "00631L".to_string(),
                PriceLimit::Range {
                    down: dec!(31.70),
                    up: dec!(47.54),
                },
            )],
        );
        assert!(share.is_valid_price("00631L", dec!(45), dec!(39.62)));
        assert!(share.is_valid_price("00631L", dec!(47.54), dec!(39.62)));
        assert!(!share.is_valid_price("00631L", dec!(47.6), dec!(39.62)));
        assert!(!share.is_valid_price("00631L", dec!(31.6), dec!(39.62)));
    }

    /// 沒有漲跌幅限制的 ETF 改用寬鬆門檻（00715L 2026-10-02 上漲 11.3%），明顯錯誤的值仍擋下。
    #[test]
    fn unlimited_securities_use_the_relaxed_tolerance() {
        use super::super::realtime::PriceLimit;

        let share = Share::new();
        let today = chrono::Local::now().date_naive();
        assert!(!share.is_valid_price("00715L", dec!(70.75), dec!(63.55)));
        share.set_price_limits(today, [("00715L".to_string(), PriceLimit::Unlimited)]);
        assert!(share.is_valid_price("00715L", dec!(70.75), dec!(63.55)));
        assert!(!share.is_valid_price("00715L", dec!(0.5), dec!(63.55)));
    }

    /// 前一天的漲跌停價不適用今天；`Unknown` 不會被記錄。
    #[test]
    fn stale_or_unknown_price_limits_are_ignored() {
        use super::super::realtime::PriceLimit;

        let share = Share::new();
        let today = chrono::Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        share.set_price_limits(
            yesterday,
            [(
                "00631L".to_string(),
                PriceLimit::Range {
                    down: dec!(31.70),
                    up: dec!(47.54),
                },
            )],
        );
        assert_eq!(share.price_limit_on("00631L", today), PriceLimit::Unknown);
        assert!(!share.is_valid_price("00631L", dec!(45), dec!(39.62)));

        share.set_price_limits(today, [("2330".to_string(), PriceLimit::Unknown)]);
        assert_eq!(share.price_limit_on("2330", today), PriceLimit::Unknown);
    }

    /// 來源已知是興櫃時（Yahoo 興櫃類股），主檔沒有該代號也適用興櫃閾值。
    #[test]
    fn is_valid_price_for_market_uses_caller_hint() {
        let share = Share::new();
        assert!(share.is_valid_price_for_market("7934", dec!(630), dec!(552.86), true));
        assert!(!share.is_valid_price_for_market("7934", dec!(630), dec!(552.86), false));
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
