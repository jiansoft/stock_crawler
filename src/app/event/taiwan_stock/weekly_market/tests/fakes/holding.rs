//! 持股基本面、主力期間合計與市場估值的假倉儲。

use super::*;

/// 回傳固定持股基本面與主力期間合計的假倉儲；`fail` 時查詢失敗。
#[derive(Default)]
pub(in super::super) struct FakePortfolio {
    pub(in super::super) fundamentals: Vec<HoldingFundamentals>,
    pub(in super::super) brokers: Vec<BrokerPeriodNet>,
    pub(in super::super) fail: bool,
}

impl FakePortfolio {
    pub(in super::super) fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }
}

#[async_trait]
impl HoldingBrokerQuery for FakePortfolio {
    async fn fetch_broker_dates(&self, _until: NaiveDate, _limit: usize) -> Result<Vec<NaiveDate>> {
        unreachable!("週摘要沿用法人一段的期間，不查主力交易日")
    }

    async fn fetch_broker_period(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        symbols: &[String],
    ) -> Result<Vec<BrokerPeriodNet>> {
        assert_eq!((from, to), (day(12), day(16)), "沿用法人一段的期間");
        assert_eq!(symbols, ["1101", "2317", "2330", "2454"]);
        if self.fail {
            return Err(anyhow!("broker down"));
        }
        Ok(self.brokers.clone())
    }
}

#[async_trait]
impl HoldingFundamentalsQuery for FakePortfolio {
    async fn fetch_holding_fundamentals(
        &self,
        symbols: &[String],
    ) -> Result<Vec<HoldingFundamentals>> {
        assert_eq!(symbols, ["1101", "2317", "2330", "2454"]);
        if self.fail {
            return Err(anyhow!("portfolio down"));
        }
        Ok(self.fundamentals.clone())
    }
}

/// 回傳固定每日估值的假倉儲；`fail` 時查詢失敗。查詢期間必須是資料日往回 5 年。
#[derive(Default)]
pub(in super::super) struct FakeValuation {
    pub(in super::super) days: Vec<MarketValuationDay>,
    pub(in super::super) fail: bool,
}

impl FakeValuation {
    pub(in super::super) fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }
}

#[async_trait]
impl MarketValuationRepository for FakeValuation {
    async fn refresh_market_valuation(&self, _from: NaiveDate, _to: NaiveDate) -> Result<u64> {
        Ok(0)
    }

    async fn fetch_market_valuation(
        &self,
        market_id: i32,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<MarketValuationDay>> {
        assert_eq!((market_id, to), (0, day(16)));
        assert_eq!(
            from,
            NaiveDate::from_ymd_opt(2021, 10, 16).expect("合法日期")
        );
        if self.fail {
            return Err(anyhow!("valuation down"));
        }
        Ok(self.days.clone())
    }
}

/// 2330 營業虧損、2317 沒有警示。
pub(in super::super) fn portfolio() -> FakePortfolio {
    use crate::domain::portfolio::health::FundamentalSignals;
    let item = |symbol: &str, operating_margin: f64| HoldingFundamentals {
        stock_symbol: symbol.to_owned(),
        signals: FundamentalSignals {
            operating_margin: Some(operating_margin),
            ..Default::default()
        },
        ..Default::default()
    };
    FakePortfolio {
        fundamentals: vec![item("2317", 3.0), item("2330", -2.5)],
        ..FakePortfolio::default()
    }
}
