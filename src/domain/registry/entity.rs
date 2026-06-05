use crate::domain::events::DomainEvent;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;

/// <summary>
/// 證券代碼值物件 (Value Object)。
/// 封裝證券代碼的字串表示與其特定商業規則判定。
/// </summary>
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StockSymbol(pub String);

impl StockSymbol {
    /// <summary>
    /// 判定是否為特別股或非普通股。
    /// 規則：若代碼中包含任何英文字母，則視為特別股。
    /// </summary>
    pub fn is_preference(&self) -> bool {
        self.0
            .chars()
            .any(|c| c.is_ascii_uppercase() || c.is_ascii_lowercase())
    }

    /// <summary>
    /// 判定是否為臺灣存託憑證 (TDR)。
    /// 規則：以 "91" 開頭的代碼通常為 TDR。
    /// </summary>
    pub fn is_tdr(&self) -> bool {
        self.0.starts_with("91")
    }
}

/// <summary>
/// 證券主檔聚合根 (Aggregate Root)。
/// 管理個股的身份識別、下市狀態、每股淨值及基本持股權重等狀態變更，並負責收集發生的領域事件。
/// </summary>
#[derive(Debug, Clone)]
pub struct Stock {
    symbol: StockSymbol,
    name: String,
    suspend_listing: bool,
    net_asset_value_per_share: Decimal,
    weight: Decimal,
    return_on_equity: Decimal,
    created_time: DateTime<Local>,
    market_id: i32,
    industry_id: i32,
    issued_share: i64,
    qfii_shares_held: i64,
    qfii_share_holding_percentage: Decimal,

    /// 用於收集聚合根內發生的領域事件。
    events: Vec<DomainEvent>,
}

impl Stock {
    /// <summary>
    /// 建立全新註冊股票的工廠方法。
    /// 此方法會自動觸發 `DomainEvent::StockRegistered` 事件。
    /// </summary>
    pub fn register(symbol: String, name: String, market_id: i32, industry_id: i32) -> Self {
        let occurred_at = Local::now();
        let mut stock = Stock {
            symbol: StockSymbol(symbol.clone()),
            name: name.clone(),
            suspend_listing: false,
            net_asset_value_per_share: Decimal::ZERO,
            weight: Decimal::ZERO,
            return_on_equity: Decimal::ZERO,
            created_time: occurred_at,
            market_id,
            industry_id,
            issued_share: 0,
            qfii_shares_held: 0,
            qfii_share_holding_percentage: Decimal::ZERO,
            events: Vec::new(),
        };

        stock.events.push(DomainEvent::StockRegistered {
            symbol,
            name,
            market_id,
            industry_id,
            occurred_at,
        });

        stock
    }

    /// <summary>
    /// 重建現有股票實體的工廠方法 (Reconstitution)。
    /// 用於從持久化介質 (如資料庫) 還原狀態，不會觸發任何領域事件。
    /// </summary>
    #[allow(clippy::too_many_arguments)]
    pub fn reconstitute(
        symbol: String,
        name: String,
        suspend_listing: bool,
        net_asset_value_per_share: Decimal,
        weight: Decimal,
        return_on_equity: Decimal,
        created_time: DateTime<Local>,
        market_id: i32,
        industry_id: i32,
        issued_share: i64,
        qfii_shares_held: i64,
        qfii_share_holding_percentage: Decimal,
    ) -> Self {
        Stock {
            symbol: StockSymbol(symbol),
            name,
            suspend_listing,
            net_asset_value_per_share,
            weight,
            return_on_equity,
            created_time,
            market_id,
            industry_id,
            issued_share,
            qfii_shares_held,
            qfii_share_holding_percentage,
            events: Vec::new(),
        }
    }

    /// <summary>
    /// 業務方法：變更個股基本識別資訊。
    /// 當有欄位變更時，會自動收集 `DomainEvent::StockIdentityChanged` 事件。
    /// </summary>
    pub fn change_identity(&mut self, name: String, market_id: i32, industry_id: i32) {
        if self.name != name || self.market_id != market_id || self.industry_id != industry_id {
            let old_name = self.name.clone();
            let old_market = self.market_id;
            let old_industry = self.industry_id;

            self.name = name.clone();
            self.market_id = market_id;
            self.industry_id = industry_id;

            self.events.push(DomainEvent::StockIdentityChanged {
                symbol: self.symbol.0.clone(),
                old_name,
                new_name: name,
                old_market_id: old_market,
                new_market_id: market_id,
                old_industry_id: old_industry,
                new_industry_id: industry_id,
                occurred_at: Local::now(),
            });
        }
    }

    /// <summary>
    /// 業務方法：更新每股淨值。
    /// 當每股淨值變更時，會自動收集 `DomainEvent::NetAssetValueUpdated` 事件。
    /// </summary>
    pub fn update_net_asset_value(&mut self, new_nav: Decimal) {
        if self.net_asset_value_per_share != new_nav {
            let old_nav = self.net_asset_value_per_share;
            self.net_asset_value_per_share = new_nav;
            self.events.push(DomainEvent::NetAssetValueUpdated {
                symbol: self.symbol.0.clone(),
                old_nav,
                new_nav,
                occurred_at: Local::now(),
            });
        }
    }

    /// <summary>
    /// 業務方法：更新下市/暫停上市狀態。
    /// </summary>
    pub fn update_suspension(&mut self, suspend: bool) {
        self.suspend_listing = suspend;
    }

    /// <summary>
    /// 業務方法：更新外資持股狀況。
    /// </summary>
    pub fn update_qfii(&mut self, shares_held: i64, percentage: Decimal) {
        self.qfii_shares_held = shares_held;
        self.qfii_share_holding_percentage = percentage;
    }

    /// <summary>
    /// 業務方法：更新已發行股數。
    /// </summary>
    pub fn update_issued_shares(&mut self, shares: i64) {
        self.issued_share = shares;
    }

    /// <summary>
    /// 業務方法：更新權值占比。
    /// </summary>
    pub fn update_weight(&mut self, weight: Decimal) {
        self.weight = weight;
    }

    /// <summary>
    /// 業務方法：更新股東權益報酬率 (ROE)。
    /// </summary>
    pub fn update_roe(&mut self, roe: Decimal) {
        self.return_on_equity = roe;
    }

    /// <summary>
    /// 提取並清除聚合根目前所收集的所有領域事件。
    /// </summary>
    pub fn pull_events(&mut self) -> Vec<DomainEvent> {
        std::mem::take(&mut self.events)
    }

    // Getters
    pub fn symbol(&self) -> &StockSymbol {
        &self.symbol
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn suspend_listing(&self) -> bool {
        self.suspend_listing
    }
    pub fn net_asset_value_per_share(&self) -> Decimal {
        self.net_asset_value_per_share
    }
    pub fn weight(&self) -> Decimal {
        self.weight
    }
    pub fn return_on_equity(&self) -> Decimal {
        self.return_on_equity
    }
    pub fn created_time(&self) -> DateTime<Local> {
        self.created_time
    }
    pub fn market_id(&self) -> i32 {
        self.market_id
    }
    pub fn industry_id(&self) -> i32 {
        self.industry_id
    }
    pub fn issued_share(&self) -> i64 {
        self.issued_share
    }
    pub fn qfii_shares_held(&self) -> i64 {
        self.qfii_shares_held
    }
    pub fn qfii_share_holding_percentage(&self) -> Decimal {
        self.qfii_share_holding_percentage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_stock_symbol_is_preference() {
        assert!(StockSymbol("2881A".to_string()).is_preference());
        assert!(StockSymbol("2330B".to_string()).is_preference());
        assert!(!StockSymbol("2330".to_string()).is_preference());
        assert!(!StockSymbol("0050".to_string()).is_preference());
    }

    #[test]
    fn test_stock_symbol_is_tdr() {
        assert!(StockSymbol("9105".to_string()).is_tdr());
        assert!(StockSymbol("9136".to_string()).is_tdr());
        assert!(!StockSymbol("2330".to_string()).is_tdr());
    }

    #[test]
    fn test_stock_register() {
        let stock = Stock::register("2330".to_string(), "台積電".to_string(), 2, 24);

        assert_eq!(stock.symbol().0, "2330");
        assert_eq!(stock.name(), "台積電");
        assert_eq!(stock.market_id(), 2);
        assert_eq!(stock.industry_id(), 24);
        assert_eq!(stock.net_asset_value_per_share(), Decimal::ZERO);
        assert!(!stock.suspend_listing());

        let mut events = stock.clone().pull_events();
        assert_eq!(events.len(), 1);
        if let Some(DomainEvent::StockRegistered {
            symbol,
            name,
            market_id,
            industry_id,
            ..
        }) = events.pop()
        {
            assert_eq!(symbol, "2330");
            assert_eq!(name, "台積電");
            assert_eq!(market_id, 2);
            assert_eq!(industry_id, 24);
        } else {
            panic!("Expected StockRegistered event");
        }
    }

    #[test]
    fn test_stock_reconstitute() {
        let created_time = Local::now();
        let stock = Stock::reconstitute(
            "2330".to_string(),
            "台積電".to_string(),
            true,
            dec!(95.12),
            dec!(31.5),
            dec!(25.7),
            created_time,
            2,
            24,
            25000,
            12000,
            dec!(48.5),
        );

        assert_eq!(stock.symbol().0, "2330");
        assert_eq!(stock.name(), "台積電");
        assert!(stock.suspend_listing());
        assert_eq!(stock.net_asset_value_per_share(), dec!(95.12));
        assert_eq!(stock.weight(), dec!(31.5));
        assert_eq!(stock.return_on_equity(), dec!(25.7));
        assert_eq!(stock.created_time(), created_time);
        assert_eq!(stock.market_id(), 2);
        assert_eq!(stock.industry_id(), 24);
        assert_eq!(stock.issued_share(), 25000);
        assert_eq!(stock.qfii_shares_held(), 12000);
        assert_eq!(stock.qfii_share_holding_percentage(), dec!(48.5));

        // Reconstitution must not generate any domain events
        assert!(stock.clone().pull_events().is_empty());
    }

    #[test]
    fn test_stock_change_identity() {
        let mut stock = Stock::register("2330".to_string(), "台積電".to_string(), 2, 24);
        let _ = stock.pull_events(); // Clear registered event

        // Change with identical values should not trigger event
        stock.change_identity("台積電".to_string(), 2, 24);
        assert!(stock.pull_events().is_empty());

        // Change with different values should trigger event
        stock.change_identity("台積電新".to_string(), 3, 25);
        assert_eq!(stock.name(), "台積電新");
        assert_eq!(stock.market_id(), 3);
        assert_eq!(stock.industry_id(), 25);

        let mut events = stock.pull_events();
        assert_eq!(events.len(), 1);
        if let Some(DomainEvent::StockIdentityChanged {
            symbol,
            old_name,
            new_name,
            old_market_id,
            new_market_id,
            old_industry_id,
            new_industry_id,
            ..
        }) = events.pop()
        {
            assert_eq!(symbol, "2330");
            assert_eq!(old_name, "台積電");
            assert_eq!(new_name, "台積電新");
            assert_eq!(old_market_id, 2);
            assert_eq!(new_market_id, 3);
            assert_eq!(old_industry_id, 24);
            assert_eq!(new_industry_id, 25);
        } else {
            panic!("Expected StockIdentityChanged event");
        }
    }

    #[test]
    fn test_stock_update_net_asset_value() {
        let mut stock = Stock::register("2330".to_string(), "台積電".to_string(), 2, 24);
        let _ = stock.pull_events(); // Clear registered event

        // Same NAV should not trigger event
        stock.update_net_asset_value(Decimal::ZERO);
        assert!(stock.pull_events().is_empty());

        // Different NAV should trigger event
        stock.update_net_asset_value(dec!(95.12));
        assert_eq!(stock.net_asset_value_per_share(), dec!(95.12));

        let mut events = stock.pull_events();
        assert_eq!(events.len(), 1);
        if let Some(DomainEvent::NetAssetValueUpdated {
            symbol,
            old_nav,
            new_nav,
            ..
        }) = events.pop()
        {
            assert_eq!(symbol, "2330");
            assert_eq!(old_nav, Decimal::ZERO);
            assert_eq!(new_nav, dec!(95.12));
        } else {
            panic!("Expected NetAssetValueUpdated event");
        }
    }

    #[test]
    fn test_stock_other_updates() {
        let mut stock = Stock::register("2330".to_string(), "台積電".to_string(), 2, 24);

        stock.update_suspension(true);
        assert!(stock.suspend_listing());

        stock.update_qfii(12000, dec!(48.5));
        assert_eq!(stock.qfii_shares_held(), 12000);
        assert_eq!(stock.qfii_share_holding_percentage(), dec!(48.5));

        stock.update_issued_shares(25000);
        assert_eq!(stock.issued_share(), 25000);

        stock.update_weight(dec!(31.5));
        assert_eq!(stock.weight(), dec!(31.5));

        stock.update_roe(dec!(25.7));
        assert_eq!(stock.return_on_equity(), dec!(25.7));
    }
}
