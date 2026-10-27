use rust_decimal_macros::dec;

use super::*;

/// 主力進出轉成入庫格式時保留合計列與各分點，主力比重由分點比重相減。
#[test]
fn broker_flow_records_keep_totals_and_brokers() {
    let broker = |name: &str, net, share| BrokerNet {
        name: name.to_string(),
        buy: net + 10,
        sell: 10,
        net,
        share,
    };
    let flow = BrokerFlow {
        stock_symbol: "2884".to_string(),
        date: NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
        buyers: vec![broker("凱基-台北", 500, dec!(6.98))],
        sellers: vec![broker("元大-總公司", 300, dec!(2.5))],
        buy_total: 500,
        sell_total: 300,
    };

    let records = broker_flow_records(&[flow]);

    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!((record.buy_total, record.sell_total), (500, 300));
    assert_eq!(record.main_share, dec!(4.48));
    assert_eq!(record.buyers[0].name, "凱基-台北");
    assert_eq!(record.sellers[0].net, 300);
}

/// 四個來源的結果合併成入庫記錄：失敗的來源只計數；四個全部失敗回錯誤。
#[test]
fn collect_daily_merges_sources_and_counts_failures() {
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
    let flows = |symbol: &str| {
        Ok(HashMap::from([(
            symbol.to_string(),
            InstitutionalFlow {
                foreign: 100,
                trust: -20,
                dealer: 5,
            },
        )]))
    };
    let margins = || {
        Ok(HashMap::from([(
            "2330".to_string(),
            MarginBalance {
                margin_previous: 10,
                margin_today: 12,
                short_previous: 3,
                short_today: 1,
            },
        )]))
    };

    let (summary, institutional, margin) = collect_daily(
        date,
        [flows("2330"), flows("6488")],
        [margins(), Err(anyhow::anyhow!("逾時"))],
    )
    .expect("部分失敗不是錯誤");
    assert_eq!(
        summary,
        DailySummary {
            institutional: 2,
            margin: 1,
            failed_sources: 1,
            rows_written: 0,
            streaks_updated: 0
        }
    );
    assert_eq!(institutional.len(), 2);
    assert!(
        institutional
            .iter()
            .any(|net| net.stock_symbol == "6488" && net.trust == -20)
    );
    assert_eq!(margin[0].margin_balance, 12);

    fn failed<T>() -> Result<T> {
        Err(anyhow::anyhow!("封鎖"))
    }
    let err = collect_daily(date, [failed(), failed()], [failed(), failed()])
        .expect_err("四個來源都失敗");
    assert!(err.to_string().contains("四個來源全部抓取失敗"), "{err}");
}

/// 融資融券欄位對應：今日餘額寫到 balance、前日餘額寫到 previous。
#[test]
fn margin_records_map_today_and_previous() {
    let map = HashMap::from([(
        "2330".to_string(),
        MarginBalance {
            margin_previous: 10,
            margin_today: 12,
            short_previous: 3,
            short_today: 1,
        },
    )]);

    assert_eq!(
        margin_records(map),
        [MarginRecord {
            stock_symbol: "2330".to_string(),
            margin_previous: 10,
            margin_balance: 12,
            short_previous: 3,
            short_balance: 1,
        }]
    );
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

/// 逐年切段：頭尾不足一年照實際起訖，同一年只有一段，起日晚於迄日沒有任何段。
#[test]
fn yearly_chunks_split_by_calendar_year() {
    assert_eq!(
        yearly_chunks(ymd(2024, 3, 5), ymd(2026, 2, 1)),
        [
            (ymd(2024, 3, 5), ymd(2024, 12, 31)),
            (ymd(2025, 1, 1), ymd(2025, 12, 31)),
            (ymd(2026, 1, 1), ymd(2026, 2, 1)),
        ]
    );
    assert_eq!(
        yearly_chunks(ymd(2026, 10, 15), ymd(2026, 10, 15)),
        [(ymd(2026, 10, 15), ymd(2026, 10, 15))]
    );
    assert!(yearly_chunks(ymd(2026, 10, 16), ymd(2026, 10, 15)).is_empty());
}

/// 只記錄連續天數重算區間的假倉儲；其餘方法不應被呼叫。
#[derive(Default)]
struct StreakRecorder {
    calls: std::sync::Mutex<Vec<(NaiveDate, NaiveDate)>>,
}

#[async_trait::async_trait]
impl ChipRepository for StreakRecorder {
    async fn save_institutional(&self, _: NaiveDate, _: &[InstitutionalNet]) -> Result<u64> {
        unreachable!("不應寫入三大法人")
    }

    async fn save_margin(&self, _: NaiveDate, _: &[MarginRecord]) -> Result<u64> {
        unreachable!("不應寫入融資融券")
    }

    async fn refresh_streaks(&self, from: NaiveDate, to: NaiveDate) -> Result<u64> {
        self.calls.lock().expect("測試鎖").push((from, to));
        Ok(10)
    }

    async fn save_holder_distributions(&self, _: &[HolderDistributionRecord]) -> Result<u64> {
        unreachable!("不應寫入集保")
    }

    async fn save_holder_levels(&self, _: &[HolderLevelRecord]) -> Result<u64> {
        unreachable!("不應寫入集保分級")
    }

    async fn replace_insider_holdings(&self, _: &[InsiderHoldingRecord]) -> Result<u64> {
        unreachable!("不應寫入董監持股")
    }

    async fn save_broker_flows(&self, _: &[BrokerFlowRecord]) -> Result<u64> {
        unreachable!("不應寫入分點")
    }
}

/// 逐年依序重算並加總更新列數。
#[tokio::test]
async fn refresh_streaks_since_runs_each_year_in_order() {
    let repo = StreakRecorder::default();
    let updated = refresh_streaks_since(&repo, ymd(2025, 6, 1), ymd(2026, 10, 15))
        .await
        .expect("成功");
    assert_eq!(updated, 20);
    assert_eq!(
        repo.calls.lock().expect("測試鎖").clone(),
        [
            (ymd(2025, 6, 1), ymd(2025, 12, 31)),
            (ymd(2026, 1, 1), ymd(2026, 10, 15)),
        ]
    );
}

/// 記錄集保寫入順序與內容的倉儲。
#[derive(Default)]
struct HolderRecorder {
    calls: std::sync::Mutex<Vec<&'static str>>,
    summaries: std::sync::Mutex<Vec<HolderDistributionRecord>>,
    levels: std::sync::Mutex<Vec<HolderLevelRecord>>,
}

#[async_trait::async_trait]
impl ChipRepository for HolderRecorder {
    async fn save_institutional(&self, _: NaiveDate, _: &[InstitutionalNet]) -> Result<u64> {
        unreachable!("不應寫入三大法人")
    }

    async fn save_margin(&self, _: NaiveDate, _: &[MarginRecord]) -> Result<u64> {
        unreachable!("不應寫入融資融券")
    }

    async fn refresh_streaks(&self, _: NaiveDate, _: NaiveDate) -> Result<u64> {
        unreachable!("不應重算連續天數")
    }

    async fn save_holder_distributions(&self, records: &[HolderDistributionRecord]) -> Result<u64> {
        self.calls.lock().expect("測試鎖").push("summary");
        self.summaries
            .lock()
            .expect("測試鎖")
            .extend_from_slice(records);
        Ok(records.len() as u64)
    }

    async fn save_holder_levels(&self, records: &[HolderLevelRecord]) -> Result<u64> {
        self.calls.lock().expect("測試鎖").push("levels");
        self.levels
            .lock()
            .expect("測試鎖")
            .extend_from_slice(records);
        Ok(records.len() as u64)
    }

    async fn replace_insider_holdings(&self, _: &[InsiderHoldingRecord]) -> Result<u64> {
        unreachable!("不應寫入董監持股")
    }

    async fn save_broker_flows(&self, _: &[BrokerFlowRecord]) -> Result<u64> {
        unreachable!("不應寫入分點")
    }
}

/// 先寫由分級算出的週摘要，再原樣寫入全部分級（含沒有千張列、不進摘要的證券）。
#[tokio::test]
async fn save_holder_distributions_writes_summary_then_every_level() {
    let level = |symbol: &str, level, holders, shares, percent| HolderLevel {
        date: ymd(2026, 10, 2),
        stock_symbol: symbol.to_string(),
        level,
        holders,
        shares,
        percent,
    };
    let levels = [
        level("2330", 12, 582, 283_883_740, dec!(1.09)),
        level("2330", 15, 1_485, 21_984_287_365, dec!(84.77)),
        level("2330", 17, 3_010_913, 25_932_370_067, dec!(100.00)),
        level("9999", 1, 10, 500, dec!(0.01)),
    ];
    let repo = HolderRecorder::default();

    save_holder_distributions(&repo, &levels)
        .await
        .expect("成功");

    assert_eq!(*repo.calls.lock().expect("測試鎖"), ["summary", "levels"]);
    assert_eq!(
        *repo.summaries.lock().expect("測試鎖"),
        [HolderDistributionRecord {
            date: ymd(2026, 10, 2),
            stock_symbol: "2330".to_string(),
            major_holders: 1_485,
            major_percent: dec!(84.77),
            total_holders: 3_010_913,
        }]
    );
    let saved = repo.levels.lock().expect("測試鎖");
    assert_eq!(saved.len(), 4);
    assert_eq!(
        saved[0],
        HolderLevelRecord {
            date: ymd(2026, 10, 2),
            stock_symbol: "2330".to_string(),
            level: 12,
            holders: 582,
            shares: 283_883_740,
            percent: dec!(1.09),
        }
    );
    assert_eq!(saved[3].stock_symbol, "9999");
}
