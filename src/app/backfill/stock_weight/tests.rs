//! 個股權值比重回填的測試。

use std::sync::Mutex as StdMutex;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;
use crate::domain::registry::entity::Stock;
use crate::infra::cache::SHARE;

/// 記錄歸零與寫入的記憶體倉儲；`fail_zero` 時歸零失敗，代號 `BROKEN` 寫入失敗。
#[derive(Default)]
struct FakeWeights {
    fail_zero: bool,
    calls: StdMutex<Vec<String>>,
}

impl FakeWeights {
    fn calls(&self) -> Vec<String> {
        let mut calls = self.calls.lock().expect("calls 鎖不會中毒").clone();
        // 並行寫入的順序不固定，歸零之後的部分依代號排序再比對。
        calls[1..].sort();
        calls
    }
}

#[async_trait::async_trait]
impl StockRepository for FakeWeights {
    async fn find_by_symbol(&self, _symbol: &str) -> Result<Option<Stock>> {
        unreachable!("權值回填不查主檔")
    }

    async fn save(&self, _stock: &Stock) -> Result<()> {
        unreachable!("權值回填不寫主檔")
    }

    async fn save_qfii_batch(&self, _stocks: &[Stock]) -> Result<u64> {
        unreachable!("權值回填不寫外資持股")
    }

    async fn fetch_all_active(&self) -> Result<Vec<Stock>> {
        unreachable!("權值回填不查主檔")
    }

    async fn update_eps_and_roe(&self) -> Result<()> {
        unreachable!("權值回填不更新 EPS")
    }

    async fn fetch_net_asset_value_per_share_is_zero(&self) -> Result<Vec<Stock>> {
        unreachable!("權值回填不查淨值")
    }

    async fn fetch_stocks_without_financial_statement(
        &self,
        _year: i32,
        _quarter: &str,
    ) -> Result<Vec<String>> {
        unreachable!("權值回填不查財報")
    }

    async fn zeroed_out_weights(&self) -> Result<()> {
        if self.fail_zero {
            anyhow::bail!("歸零失敗");
        }
        self.calls
            .lock()
            .expect("calls 鎖不會中毒")
            .push("zero".to_owned());
        Ok(())
    }

    async fn update_weight(&self, stock_symbol: &str, weight: Decimal) -> Result<()> {
        if stock_symbol == "BROKEN" {
            anyhow::bail!("寫入權值失敗");
        }
        self.calls
            .lock()
            .expect("calls 鎖不會中毒")
            .push(format!("{stock_symbol}={weight}"));
        Ok(())
    }
}

fn weight(symbol: &str, weight: Decimal) -> SaveStockWeightCommand {
    SaveStockWeightCommand {
        symbol: symbol.to_owned(),
        weight,
    }
}

/// 先歸零再逐檔寫入；單檔失敗只記錄，其餘照寫。
#[tokio::test]
async fn save_weights_zeroes_then_updates_each_stock() {
    let repo = FakeWeights::default();
    let weights = [
        weight("2330", dec!(31.5)),
        weight("BROKEN", dec!(1)),
        weight("2317", dec!(4.2)),
    ];

    save_weights(&repo, &weights).await.expect("單檔失敗不中斷");

    assert_eq!(repo.calls(), ["zero", "2317=4.2", "2330=31.5"]);
}

/// 沒有抓到任何權重時不動資料庫（避免把全部權重歸零）；歸零失敗時回傳錯誤、不寫入。
#[tokio::test]
async fn save_weights_skips_empty_and_stops_when_zeroing_fails() {
    let repo = FakeWeights::default();
    save_weights(&repo, &[]).await.expect("空清單直接結束");
    assert!(repo.calls.lock().expect("calls 鎖不會中毒").is_empty());

    let failing = FakeWeights {
        fail_zero: true,
        ..Default::default()
    };
    let error = save_weights(&failing, &[weight("2330", dec!(31.5))])
        .await
        .expect_err("歸零失敗要回傳錯誤");
    assert!(format!("{error:#}").contains("歸零失敗"), "{error:#}");
    assert!(failing.calls.lock().expect("calls 鎖不會中毒").is_empty());
}

/// 手動執行：抓期交所權值比重並更新正式資料（需要網路）。
#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {
            tracing::debug!("成功執行 execute");
        }
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
}

/// 以本機 mock server 取代期交所權值頁（G15），驗證上市、上櫃兩種版面的抓取（C19）。
mod mock_sources {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const TWSE_PATH: &str = "/cht/9/futuresQADetail";
    const TPEX_PATH: &str = "/cht/2/tPEXPropertion";

    /// 一列兩檔的權值表；`nested` 為上市頁多一層 `div` 的版面。
    fn weight_page(nested: bool, cells: &str) -> String {
        let table = format!("<table><tbody><tr>{cells}</tr></tbody></table>");
        let body = if nested {
            format!("<div><div>{table}</div></div>")
        } else {
            format!("<div>{table}</div>")
        };
        format!(r#"<html><body><div id="printhere">{body}</div></body></html>"#)
    }

    fn html(body: String) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body, "text/html; charset=utf-8")
    }

    async fn mount(server: &MockServer, route: &str, response: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(response)
            .expect(1)
            .mount(server)
            .await;
    }

    /// 上市、上櫃各自依版面解析，結果收進同一個容器。
    #[tokio::test]
    async fn handle_stock_exchange_collects_both_layouts() {
        let server = MockServer::start().await;
        mount(
            &server,
            TWSE_PATH,
            html(weight_page(
                true,
                "<td>1</td><td>2330</td><td>台積電</td><td>34.61%</td>\
                 <td>2</td><td>2317</td><td>鴻海</td><td>3.50%</td>",
            )),
        )
        .await;
        mount(
            &server,
            TPEX_PATH,
            html(weight_page(
                false,
                "<td>1</td><td>6488</td><td>環球晶</td><td>5.25%</td>\
                 <td></td><td></td><td></td><td></td>",
            )),
        )
        .await;

        let collected = Arc::new(Mutex::new(Vec::new()));
        let uri = server.uri();
        let result = with_origins(&[("https://www.taifex.com.tw", &uri)], async {
            handle_stock_exchange(StockExchange::TWSE, Arc::clone(&collected)).await?;
            handle_stock_exchange(StockExchange::TPEx, Arc::clone(&collected)).await
        })
        .await;

        assert!(result.is_ok(), "{result:?}");
        let weights: Vec<(String, Decimal)> = collected
            .lock()
            .await
            .iter()
            .map(|command| (command.symbol.clone(), command.weight))
            .collect();
        assert_eq!(
            weights,
            [
                ("2330".to_owned(), dec!(34.61)),
                ("2317".to_owned(), dec!(3.50)),
                ("6488".to_owned(), dec!(5.25)),
            ],
            "上櫃尾列右半空白要略過"
        );
    }

    /// 來源回 5xx 或權值頁沒有任何資料時回錯（H38），`execute` 因此中止、不會進入全部歸零。
    #[tokio::test]
    async fn handle_stock_exchange_rejects_error_status_and_empty_page() {
        let server = MockServer::start().await;
        mount(&server, TWSE_PATH, ResponseTemplate::new(500)).await;
        mount(&server, TPEX_PATH, html(weight_page(false, ""))).await;

        let collected = Arc::new(Mutex::new(Vec::new()));
        let uri = server.uri();
        let (listed, otc) = with_origins(&[("https://www.taifex.com.tw", &uri)], async {
            (
                handle_stock_exchange(StockExchange::TWSE, Arc::clone(&collected)).await,
                handle_stock_exchange(StockExchange::TPEx, Arc::clone(&collected)).await,
            )
        })
        .await;

        let listed = format!("{:#}", listed.expect_err("上市回 500 應回錯誤"));
        assert!(listed.contains("500"), "{listed}");
        let otc = format!("{:#}", otc.expect_err("上櫃沒有資料應回錯誤"));
        assert!(otc.contains("沒有任何資料"), "{otc}");
        assert!(collected.lock().await.is_empty(), "失敗時不收任何權重");
    }
}
