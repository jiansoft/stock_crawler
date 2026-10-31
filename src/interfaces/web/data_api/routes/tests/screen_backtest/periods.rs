//! 回測主流程：換股日、相對強度排序、報酬平均、大盤與 0050 基準、年度彙總、交易成本、換股緩衝、
//! 續抱條件、產業上限、個股貢獻、目前持股與換股日偏移。

use super::*;

/// 換股日、相對強度排序取前 N 檔、沒有報酬的股票不列入平均、大盤基準、0050、年度彙總、交易成本、換股緩衝、
/// 續抱條件、產業上限、個股貢獻、目前持股與換股日偏移。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_periods_costs_and_current() {
    let Some(pool) = seeded("screen_backtest_periods_costs_and_current").await else {
        return;
    };
    // 每期取相對強度前 2 檔：03-01 BA、BC（10、2），03-08 BB、BC（BC 沒有報酬，只算 BB 的 8）。
    // 03-15 還沒有報酬，不列入。
    let (status, json) = query("limit=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["to"], "1993-03-15");
    assert_eq!(
        (json["hold_days"].as_u64(), json["limit"].as_u64()),
        (Some(5), Some(2))
    );
    assert_eq!(json["adjusted"], true);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 2, Some(8.0)),
        ]
    );
    assert_eq!(json["periods"][0]["exit_date"], "1993-03-08");
    assert_eq!(json["periods"][1]["exit_date"], "1993-03-15");
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BA", "79979BC"])
    );
    // 大盤基準：03-01 三檔平均 8/3；03-08 只有兩檔有報酬，平均 5.5。
    assert_eq!(json["periods"][0]["benchmark_return"], 2.67);
    assert_eq!(json["periods"][1]["benchmark_return"], 5.5);
    let summary = &json["summary"];
    assert_eq!(summary["periods"], 2);
    assert_eq!(summary["invested_periods"], 2);
    assert_eq!(summary["total_return"], 14.48);
    assert_eq!(summary["benchmark_total_return"], 8.31);
    assert_eq!(summary["win_rate"], 1.0);
    assert_eq!(summary["beat_market_rate"], 1.0);
    assert_eq!(summary["max_drawdown"], 0.0);
    // 0050：03-01 報酬 4%、03-08 缺值算 0；兩期都勝過 0050。
    assert_eq!(json["etf_benchmark"], "0050");
    assert_eq!(json["periods"][0]["etf_return"], 4.0);
    assert!(json["periods"][1]["etf_return"].is_null());
    assert_eq!(summary["etf_total_return"], 4.0);
    assert_eq!(summary["etf_max_drawdown"], 0.0);
    assert_eq!(summary["beat_etf_rate"], 1.0);
    // 年度彙總：兩期都在 1993 年。
    let years = json["years"].as_array().expect("years array");
    assert_eq!(years.len(), 1);
    assert_eq!(
        (years[0]["year"].as_i64(), years[0]["periods"].as_i64()),
        (Some(1993), Some(2))
    );
    assert_eq!(years[0]["total_return"], 14.48);
    assert_eq!(years[0]["benchmark_total_return"], 8.31);
    assert_eq!(years[0]["etf_total_return"], 4.0);
    // 交易成本：03-01 全部買進（0.1425%）；03-01 那期 BA 漲 10%、BC 漲 2%，期末 BA 占 0.55 ÷ 1.06。
    // 03-08 賣出 BA（0.4425%）、BC 加碼回一半與買進 BB 一半（0.1425%），兩邊都是 0.55 ÷ 1.06。
    assert_eq!(json["fee_discount"], 1.0);
    assert_eq!(json["periods"][0]["turnover"], 1.0);
    assert_eq!(json["periods"][0]["return_after_costs"], 5.86);
    assert_eq!(json["periods"][1]["turnover"], 0.5189);
    // 8 − 0.5189 × (0.4425 + 0.1425) ≈ 7.70。
    assert_eq!(json["periods"][1]["return_after_costs"], 7.7);
    assert_eq!(summary["avg_turnover"], 0.7594);
    // 1.058575 × 1.076965 − 1 ≈ 14.00%。
    assert_eq!(summary["total_return_after_costs"], 14.0);
    let (_, free) = query("limit=2&fee_discount=0").await;
    assert_eq!(free["periods"][0]["return_after_costs"], 6.0);
    // 只剩賣出 BA 的證交稅：8 − 0.5189 × 0.3。
    assert_eq!(free["periods"][1]["return_after_costs"], 7.84);

    // 換股緩衝：只持 1 檔、緩衝 3 名。03-08 的 BA 掉到第 3 名仍續抱（報酬 3），不換成第 1 名的 BB。
    let (status, json) = query("limit=1&keep_rank=3").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["keep_rank"], 3);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BA"])
    );
    assert_eq!(json["periods"][1]["turnover"], 0.0);
    // 緩衝 2 名時 BA（第 3 名）賣出、換成 BB。
    let (_, json) = query("limit=1&keep_rank=2").await;
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BB"])
    );
    // 只有兩期，報酬有波動才有夏普值。
    assert!(json["summary"]["volatility"].as_f64().is_some());
    assert!(json["summary"]["sharpe_ratio"].as_f64().is_some());

    // 續抱條件：只選 RS ≥ 85（03-01 BA、03-08 BB）。BA 03-08 的 RS 50 不再符合選股條件，
    // 但達續抱門檻 50 就續抱（報酬 3）；門檻 51 時換成 BB（報酬 8）。
    let (status, json) = query("limit=1&min_rs_score=85&hold_min_rs_score=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["hold_min_rs_score"], 50);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    let (_, json) = query("limit=1&min_rs_score=85&hold_min_rs_score=51").await;
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BB"])
    );

    // 產業上限：03-01 前兩名 BA（90）、BC（80）同為產業 24，上限 1 時跳過 BC、補產業 99 的 BB；
    // 03-08 的 BB（95，產業 99）、BC（70，產業 24）本來就不同產業。
    let (status, json) = query("limit=2&max_per_industry=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_per_industry"], 1);
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BA", "79979BB"])
    );
    assert_eq!(json["periods"][0]["avg_return"], 3.0);
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BB", "79979BC"])
    );
    let (_, json) = query("limit=2").await;
    assert!(json["max_per_industry"].is_null());

    // 個股貢獻：limit=2 時 03-01 BA 10、BC 2（各 1/2）、03-08 BB 8、BC 沒有報酬（BB 獨得）。
    let (_, json) = query("limit=2").await;
    let contributors = &json["contributors"];
    assert_eq!(contributors["held_symbols"], 3);
    assert_eq!(
        contributors["best"],
        serde_json::json!([
            {"symbol": "79979BB", "name": "79979BB", "periods_held": 1, "contribution": 8.0, "avg_return": 8.0},
            {"symbol": "79979BA", "name": "79979BA", "periods_held": 1, "contribution": 5.0, "avg_return": 10.0},
            {"symbol": "79979BC", "name": "79979BC", "periods_held": 2, "contribution": 1.0, "avg_return": 2.0},
        ])
    );
    assert_eq!(contributors["worst"], serde_json::json!([]));

    // 目前持股：03-15 還沒滿 5 日（不列入各期），三檔分數都是 50、依代號取 BA、BB；
    // 上一期的 BB 續抱、BA 新買進（換手 0.5），上一期的 BC 賣出。
    assert_eq!(
        json["current"],
        serde_json::json!({
            "date": "1993-03-15", "days_elapsed": 0, "days_to_next_rebalance": 5,
            "market_filtered": false, "turnover": 0.5, "cash": 0.0,
            // 種入的日線沒有收盤價，換股日至今的報酬都是 null。
            "return_to_date": null, "etf_return_to_date": null,
            "holdings": [
                {"symbol": "79979BA", "name": "79979BA", "weight": 50.0, "added": true, "return_to_date": null},
                {"symbol": "79979BB", "name": "79979BB", "weight": 50.0, "added": false, "return_to_date": null},
            ],
            "exited": [{"symbol": "79979BC", "name": "79979BC", "weight": 50.0}],
        })
    );
    // 迄日 03-12：03-08 的出場日為 03-15，尚未完成，不能使用已回填的未來報酬。
    let (status, json) =
        get("/api/v1/stocks/screen/backtest?from=1993-03-01&to=1993-03-12&hold_days=5&limit=2")
            .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["periods"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["current"]["date"], "1993-03-08");

    // 換股日偏移 1：換股日變成 03-02、03-09，都沒有種入報酬；03-02 不列入，03-09 是目前持股，
    // 之後經過 03-10、11、12、15 四個交易日，再 1 天換股。
    let (status, json) = query("limit=2&rebalance_offset=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rebalance_offset"], 1);
    assert_eq!(periods(&json), []);
    assert_eq!(json["current"]["date"], "1993-03-09");
    assert_eq!(json["current"]["days_elapsed"], 4);
    assert_eq!(json["current"]["days_to_next_rebalance"], 1);
    let (_, json) = query("limit=2").await;
    assert_eq!(json["rebalance_offset"], 0);
    let (status, _) = query("rebalance_offset=5").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    cleanup(pool).await;
}
