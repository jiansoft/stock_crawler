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
