//! 條件選股回測 endpoint 整合測試：條件篩選與參數驗證。

use super::*;

/// 大盤與市場廣度濾網、市場與產業篩選、排除產業、相對強度與訊號、籌碼條件，以及參數驗證。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_filters_and_validation() {
    let Some(pool) = seeded("screen_backtest_filters_and_validation").await else {
        return;
    };
    // 大盤濾網：03-08 加權指數低於 20 日均線，這期空手並賣光（扣賣出成本）。
    let (status, json) = query("limit=2&market_filter_ma=20").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["market_filter_ma"], 20);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 0, None),
        ]
    );
    assert_eq!(json["periods"][0]["market_filtered"], false);
    assert_eq!(json["periods"][1]["market_filtered"], true);
    assert_eq!(json["summary"]["market_filtered_periods"], 1);
    assert_eq!(json["summary"]["invested_periods"], 1);
    // 市場廣度濾網：門檻 50% 時 03-08（30%）空手；門檻 30% 剛好通過（等於門檻算通過）。
    let (status, json) = query("limit=2&market_filter_breadth=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["market_filter_breadth"], 50);
    assert!(json["market_filter_ma"].is_null());
    assert_eq!(json["periods"][1]["market_filtered"], true);
    assert_eq!(json["summary"]["market_filtered_periods"], 1);
    let (_, json) = query("limit=2&market_filter_breadth=30").await;
    assert_eq!(json["summary"]["market_filtered_periods"], 0);
    // 與均線濾網併用：兩者都要通過；上櫃 03-01 只有 40%、03-08 沒有統計列視為通過。
    let (_, json) = query("limit=2&market_filter_breadth=30&market_filter_ma=20").await;
    assert_eq!(json["summary"]["market_filtered_periods"], 1);
    let (_, json) = query("market=tpex&market_filter_breadth=50").await;
    assert_eq!(json["periods"][0]["market_filtered"], true);
    assert_eq!(json["summary"]["invested_periods"], 0);
    // 沒設濾網時不受加權指數影響。
    let (_, json) = query("limit=2").await;
    assert_eq!(json["summary"]["market_filtered_periods"], 0);

    // 上櫃只有 BC：03-08 母體沒有任何報酬，那一期不列入。
    let (_, json) = query("market=tpex").await;
    assert_eq!(periods(&json), [("1993-03-01".to_owned(), 1, Some(2.0))]);
    assert_eq!(json["periods"][0]["benchmark_return"], 2.0);

    // 產業篩選只影響選股：03-08 的 BA 3、BC 沒有報酬。
    let (_, json) = query("industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 2, Some(3.0)),
        ]
    );
    assert_eq!(json["periods"][1]["benchmark_return"], 5.5);
    assert!(json["exclude_industry"].is_null());

    // 排除半導體業：只剩產業 99 的 BB（03-01 −4、03-08 8），大盤基準不變。
    let (status, json) = query("limit=2&exclude_industry=%20%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD,%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["exclude_industry"], "半導體業");
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(-4.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    assert_eq!(json["periods"][1]["benchmark_return"], 5.5);

    // 相對強度門檻：03-01 BA、03-08 BB。
    let (_, json) = query("min_rs_score=85").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );

    // 訊號條件：03-01 回看視窗內沒有 KD 黃金交叉 → 空手；03-08 視窗 03-02～03-08 有 BB。
    let (_, json) = query("indicator=kd&kind=golden_cross").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 0, None),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    assert_eq!(json["periods"][0]["symbols"], serde_json::json!([]));
    assert_eq!(json["summary"]["invested_periods"], 1);
    assert_eq!(json["summary"]["total_return"], 8.0);
    assert_eq!(json["summary"]["win_rate"], 0.5);
    assert_eq!(json["indicator"], "kd");

    // 籌碼條件：外資連買 2 天，03-01 BA、03-08 BB；投信連買 1 天只有 03-08 的 BC（沒有報酬）。
    let (_, json) = query("min_foreign_streak=2").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    let (_, json) = query("min_trust_streak=1").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 0, None),
            ("1993-03-08".to_owned(), 1, None),
        ]
    );
    assert_eq!(json["summary"]["total_return"], 0.0);

    for (extra, status) in [
        ("limit=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=101", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=5&keep_rank=4", StatusCode::UNPROCESSABLE_ENTITY),
        ("keep_rank=201", StatusCode::UNPROCESSABLE_ENTITY),
        ("market_filter_ma=50", StatusCode::UNPROCESSABLE_ENTITY),
        ("market_filter_breadth=0", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "market_filter_breadth=100",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("hold_min_rs_score=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("hold_min_rs_score=100", StatusCode::UNPROCESSABLE_ENTITY),
        ("max_per_industry=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("max_per_industry=101", StatusCode::UNPROCESSABLE_ENTITY),
        ("fee_discount=1.5", StatusCode::UNPROCESSABLE_ENTITY),
        ("fee_discount=-0.1", StatusCode::UNPROCESSABLE_ENTITY),
        ("market=otc", StatusCode::UNPROCESSABLE_ENTITY),
        ("kind=breakout", StatusCode::UNPROCESSABLE_ENTITY),
        ("indicator=ma10", StatusCode::UNPROCESSABLE_ENTITY),
        ("min_rs_score=0", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "min_rs_score=90&max_rs_score=80",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("min_foreign_streak=61", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "industry=%E4%B8%8D%E5%AD%98%E5%9C%A8",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "exclude_industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD,%E4%B8%8D%E5%AD%98%E5%9C%A8",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD&exclude_industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "exclude_industry=a,b,c,d,e,f,g,h,i,j,k",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (actual, json) = query(extra).await;
        assert_eq!(actual, status, "{extra}: {json}");
        assert!(json["error"].is_string(), "{extra}: {json}");
    }
    for (path, status) in [
        ("hold_days=10", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "from=1993-03-16&to=1993-03-15",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "from=1983-03-01&to=1993-03-15",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("from=1993-3-01", StatusCode::UNPROCESSABLE_ENTITY),
        ("to=1900-01-01", StatusCode::NOT_FOUND),
    ] {
        let (actual, json) = get(&format!("/api/v1/stocks/screen/backtest?{path}")).await;
        assert_eq!(actual, status, "{path}: {json}");
    }

    cleanup(pool).await;
}
