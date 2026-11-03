use crate::infra::cache::SHARE;

// 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
use super::*;

fn stock(symbol: &str, market_id: i32) -> Stock {
    Stock::reconstitute(
        symbol.to_string(),
        format!("{symbol} 名稱"),
        false,
        rust_decimal::Decimal::ZERO,
        rust_decimal::Decimal::ZERO,
        rust_decimal::Decimal::ZERO,
        Local::now(),
        market_id,
        1,
        0,
        0,
        rust_decimal::Decimal::ZERO,
    )
}

/// 終止上市名單只標記目前仍在上市市場、尚未標記的股票；已轉上櫃或興櫃的不標記。
#[test]
fn twse_delisting_only_marks_listed_stocks() {
    assert!(should_mark_twse_delisted(&stock("1101", 2)));
    assert!(!should_mark_twse_delisted(&stock("6423", 4)));
    assert!(!should_mark_twse_delisted(&stock("6423", 5)));
    let mut marked = stock("1101", 2);
    marked.update_suspension(true);
    assert!(!should_mark_twse_delisted(&marked));
}

/// 名冊筆數低於安全下限就放棄比對。
#[test]
fn ensure_roster_size_rejects_tiny_rosters() {
    assert!(ensure_roster_size(StockExchangeMarket::Listed, MIN_ROSTER_SIZE_PER_MARKET).is_ok());
    let err = ensure_roster_size(StockExchangeMarket::OverTheCounter, 3).expect_err("太少");
    assert!(err.to_string().contains("size=3"), "{err}");
}

/// 只挑涵蓋市場內、不在名冊上的證券；超過上限整批放棄。
#[test]
fn missing_from_roster_filters_and_caps() {
    let roster: HashSet<String> = ["2330", "6488"].iter().map(|s| s.to_string()).collect();
    let covered: HashSet<i32> = [2, 4].into_iter().collect();
    let active = || {
        vec![
            stock("2330", 2),
            stock("4712", 4),
            stock("1234", 5),
            stock("6488", 4),
        ]
    };

    let missing = missing_from_roster(active(), &roster, &covered, 10).expect("未超過上限");
    assert_eq!(
        missing
            .iter()
            .map(|stock| stock.symbol().0.as_str())
            .collect::<Vec<_>>(),
        ["4712"]
    );
    assert!(
        missing_from_roster(Vec::new(), &roster, &covered, 0)
            .expect("空集合")
            .is_empty()
    );
    let err = missing_from_roster(active(), &HashSet::new(), &covered, 1).expect_err("超過上限");
    assert!(err.to_string().contains("差集達 3 檔（上限 1）"), "{err}");
}

/// 公開發行不是交易市場，不可併入差集比對的名冊。
#[test]
fn roster_markets_excludes_public_offering() {
    let markets: Vec<_> = roster_markets().collect();
    assert_eq!(
        markets,
        vec![
            StockExchangeMarket::Listed,
            StockExchangeMarket::OverTheCounter,
            StockExchangeMarket::Emerging,
        ]
    );
}

/// 記憶體版股票主檔：`fail_save` 內的代號寫入失敗，查詢代號 `BROKEN` 回錯誤。
#[derive(Default)]
struct FakeStocks {
    stocks: Vec<Stock>,
    fail_save: HashSet<String>,
    saved: std::sync::Mutex<Vec<Stock>>,
}

impl FakeStocks {
    fn new(stocks: Vec<Stock>, fail_save: &[&str]) -> Self {
        Self {
            stocks,
            fail_save: fail_save.iter().map(|symbol| symbol.to_string()).collect(),
            ..Default::default()
        }
    }

    /// 寫入過的（代號, 是否下市）。
    fn saved(&self) -> Vec<(String, bool)> {
        self.saved
            .lock()
            .expect("saved 鎖不會中毒")
            .iter()
            .map(|stock| (stock.symbol().0.clone(), stock.suspend_listing()))
            .collect()
    }
}

#[async_trait::async_trait]
impl StockRepository for FakeStocks {
    async fn find_by_symbol(&self, symbol: &str) -> Result<Option<Stock>> {
        if symbol == "BROKEN" {
            anyhow::bail!("查詢股票主檔失敗");
        }
        Ok(self
            .stocks
            .iter()
            .find(|stock| stock.symbol().0 == symbol)
            .cloned())
    }

    async fn save(&self, stock: &Stock) -> Result<()> {
        if self.fail_save.contains(&stock.symbol().0) {
            anyhow::bail!("寫入股票主檔失敗");
        }
        self.saved
            .lock()
            .expect("saved 鎖不會中毒")
            .push(stock.clone());
        Ok(())
    }

    async fn save_qfii_batch(&self, _stocks: &[Stock]) -> Result<u64> {
        unreachable!("下市回填不寫外資持股")
    }

    async fn fetch_all_active(&self) -> Result<Vec<Stock>> {
        Ok(self
            .stocks
            .iter()
            .filter(|stock| !stock.suspend_listing())
            .cloned()
            .collect())
    }

    async fn update_eps_and_roe(&self) -> Result<()> {
        unreachable!("下市標記不更新 EPS")
    }

    async fn fetch_net_asset_value_per_share_is_zero(&self) -> Result<Vec<Stock>> {
        unreachable!("下市標記不查淨值")
    }

    async fn fetch_stocks_without_financial_statement(
        &self,
        _year: i32,
        _quarter: &str,
    ) -> Result<Vec<String>> {
        unreachable!("下市標記不查財報")
    }

    async fn zeroed_out_weights(&self) -> Result<()> {
        unreachable!("下市標記不動權值")
    }

    async fn update_weight(
        &self,
        _stock_symbol: &str,
        _weight: rust_decimal::Decimal,
    ) -> Result<()> {
        unreachable!("下市標記不動權值")
    }
}

fn suspended(symbol: &str, market_id: i32) -> Stock {
    let mut stock = stock(symbol, market_id);
    stock.update_suspension(true);
    stock
}

fn suspend_listing(symbol: &str, delisting_date: &str) -> SuspendListing {
    SuspendListing {
        delisting_date: delisting_date.to_string(),
        name: format!("{symbol} 名稱"),
        stock_symbol: symbol.to_string(),
    }
}

/// 終止上市名單：只標記仍在上市市場、未下市的股票；民國 110 年前、查無、已轉上櫃、
/// 已標記的都略過，寫入失敗只記錄不中斷。
#[tokio::test]
async fn mark_twse_delisted_marks_listed_stocks_only() {
    let repo = FakeStocks::new(
        vec![
            stock("1101", 2),
            stock("1102", 2),
            stock("2002", 2),
            stock("6423", 4),
            suspended("1103", 2),
        ],
        &["2002"],
    );
    let delisted = [
        suspend_listing("1101", "1150301"),
        suspend_listing("1102", "1091231"),
        suspend_listing("2002", "1150301"),
        suspend_listing("6423", "1150301"),
        suspend_listing("1103", "1150301"),
        suspend_listing("9999", "1150301"),
    ];

    let marked = mark_twse_delisted(&repo, &delisted)
        .await
        .expect("查詢都成功");

    assert_eq!(marked, 1);
    assert_eq!(repo.saved(), [("1101".to_string(), true)]);
}

/// 查詢股票主檔失敗時整個階段回錯誤，不再往下處理。
#[tokio::test]
async fn mark_twse_delisted_stops_when_lookup_fails() {
    let repo = FakeStocks::new(vec![stock("1101", 2)], &[]);
    let delisted = [
        suspend_listing("BROKEN", "1150301"),
        suspend_listing("1101", "1150301"),
    ];

    let err = mark_twse_delisted(&repo, &delisted)
        .await
        .expect_err("查詢失敗要往上傳");

    assert!(err.to_string().contains("查詢股票主檔失敗"), "{err}");
    assert!(repo.saved().is_empty());
}

/// 每個名冊市場各 `MIN_ROSTER_SIZE_PER_MARKET` 檔假代號，再加上指定的真代號。
fn roster(market: StockExchangeMarket, extra: &[&str]) -> HashSet<String> {
    (0..MIN_ROSTER_SIZE_PER_MARKET)
        .map(|index| format!("{}-{index}", market.serial()))
        .chain(extra.iter().map(|symbol| symbol.to_string()))
        .collect()
}

/// 依市場回傳名冊並記下抓取順序；`small`／`broken` 指定的市場回過小名冊或錯誤。
fn roster_source(
    calls: &mut Vec<StockExchangeMarket>,
    small: Option<StockExchangeMarket>,
    broken: Option<StockExchangeMarket>,
) -> impl FnMut(StockExchangeMarket) -> std::future::Ready<Result<HashSet<String>>> + '_ {
    move |market| {
        calls.push(market);
        let result = if Some(market) == broken {
            Err(anyhow::anyhow!("名冊頁面逾時"))
        } else if Some(market) == small {
            Ok(["2330".to_string()].into_iter().collect())
        } else {
            Ok(roster(market, &["2330", "6488"]))
        };
        std::future::ready(result)
    }
}

/// 名冊差集：涵蓋市場內、未下市卻不在名冊上的才標記；名冊外的市場與已下市的不動，
/// 寫入失敗的不計入。
#[tokio::test]
async fn mark_missing_from_rosters_marks_symbols_off_the_roster() {
    let repo = FakeStocks::new(
        vec![
            stock("2330", 2),
            stock("6488", 4),
            stock("4712", 4),
            stock("1234", 5),
            stock("7777", 9),
            suspended("8888", 2),
        ],
        &["1234"],
    );
    let mut calls = Vec::new();

    let marked = mark_missing_from_rosters(&repo, roster_source(&mut calls, None, None), 10)
        .await
        .expect("名冊都正常");

    assert_eq!(marked, 1);
    assert_eq!(repo.saved(), [("4712".to_string(), true)]);
    assert_eq!(
        calls,
        [
            StockExchangeMarket::Listed,
            StockExchangeMarket::OverTheCounter,
            StockExchangeMarket::Emerging,
        ]
    );
}

/// 三道防線：名冊抓取失敗、名冊過小、差集超過上限都放棄，一檔都不寫。
#[tokio::test]
async fn mark_missing_from_rosters_gives_up_on_suspicious_rosters() {
    let active = || vec![stock("2330", 2), stock("4712", 4), stock("1234", 5)];

    let repo = FakeStocks::new(active(), &[]);
    let mut calls = Vec::new();
    let source = roster_source(&mut calls, None, Some(StockExchangeMarket::OverTheCounter));
    let err = mark_missing_from_rosters(&repo, source, 10)
        .await
        .expect_err("抓取失敗");
    assert!(
        err.to_string().contains("fetch isin roster failed"),
        "{err}"
    );
    assert_eq!(calls.len(), 2, "失敗後不再抓下一個市場");

    let mut calls = Vec::new();
    let source = roster_source(&mut calls, Some(StockExchangeMarket::Emerging), None);
    let err = mark_missing_from_rosters(&repo, source, 10)
        .await
        .expect_err("名冊過小");
    assert!(err.to_string().contains("size=1"), "{err}");

    let mut calls = Vec::new();
    let err = mark_missing_from_rosters(&repo, roster_source(&mut calls, None, None), 1)
        .await
        .expect_err("差集超過上限");
    assert!(err.to_string().contains("差集達 2 檔（上限 1）"), "{err}");

    assert!(repo.saved().is_empty());
}

#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {
            tracing::debug!("execute executed successfully.");
        }
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
}

/// 手動入口：以 ISIN 名冊差集清理歷史積欠的未標記下市證券。
///
/// 首次執行時，多年未被標記的終止上櫃與 ETF 清算標的可能超過排程的
/// 單輪上限（[`MAX_DELIST_PER_RUN`]），此測試以放寬的上限一次清完。
/// 執行前建議先跑一次並檢查 log 中列出的標的是否合理。
#[tokio::test]
#[ignore = "live test：連線真實外部網站並寫入資料庫，需要時手動執行"]
async fn test_mark_stocks_missing_from_isin_roster_manual_cleanup() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    match mark_stocks_missing_from_isin_roster(1000).await {
        Ok(marked) => println!("已標記 {marked} 檔下市證券"),
        Err(why) => println!("差集清理失敗: {why:#}"),
    }
}
