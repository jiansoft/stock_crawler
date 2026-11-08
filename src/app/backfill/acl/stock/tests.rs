//! 股票主檔防腐層轉換的測試。

use super::*;

#[test]
fn test_to_registration_command_success() {
    let isin = InternationalSecuritiesIdentificationNumber {
        stock_symbol: "2330".to_string(),
        name: "台積電".to_string(),
        isin_code: "TW0002330008".to_string(),
        listing_date: "1994/09/05".to_string(),
        industry: "半導體業".to_string(),
        cfi_code: "ESVUFR".to_string(),
        market: crate::core::declare::StockExchangeMarket::Listed,
    };

    let cmd = IsinAclMapper::from_isin(&isin);
    assert!(cmd.is_some());
    let cmd = cmd.unwrap();
    assert_eq!(cmd.symbol, "2330");
    assert_eq!(cmd.name, "台積電");
    assert_eq!(cmd.market_id, 2);
    assert_eq!(cmd.industry_id, 24);
}

#[test]
fn test_to_registration_command_invalid_industry() {
    let isin = InternationalSecuritiesIdentificationNumber {
        stock_symbol: "0050".to_string(),
        name: "元大台灣50".to_string(),
        isin_code: "TW0000050004".to_string(),
        listing_date: "2003/06/30".to_string(),
        industry: "ETF".to_string(),
        cfi_code: "CEOGEU".to_string(),
        market: crate::core::declare::StockExchangeMarket::Listed,
    };

    let cmd = IsinAclMapper::from_isin(&isin);
    assert!(cmd.is_none());
}

/// 特別股代號辨識：4 碼數字後接大寫字母；普通股、ETF、太短的代號都不是。
#[test]
fn preferred_parent_detects_preferred_shares() {
    assert_eq!(preferred_parent("1101B"), Some("1101"));
    assert_eq!(preferred_parent("2887Z1"), Some("2887"));
    for symbol in [
        "2330",
        "00878",
        "0050",
        "910322",
        "11A",
        "AB12C",
        "台泥乙特",
    ] {
        assert_eq!(preferred_parent(symbol), None, "{symbol}");
    }
}

/// 未分類的特別股沿用普通股產業；其他產業與查不到普通股時維持原值。
#[test]
fn resolve_industry_uses_parent_for_uncategorized_preferred() {
    let parent = |symbol: &str| match symbol {
        "1101" => Some(1),
        "9999" => Some(99),
        _ => None,
    };
    assert_eq!(resolve_industry(99, "1101B", parent), 1);
    assert_eq!(resolve_industry(24, "1101B", parent), 24);
    assert_eq!(resolve_industry(99, "6592B", parent), 99);
    assert_eq!(resolve_industry(99, "9999A", parent), 99);
    assert_eq!(resolve_industry(99, "2330", parent), 99);
}

/// TDR 的「存託憑證」產業可對應到產業代碼 39；快取沒有普通股的特別股維持未分類。
#[test]
fn test_to_registration_command_maps_tdr_and_preferred() {
    let isin = |symbol: &str, industry: &str| InternationalSecuritiesIdentificationNumber {
        stock_symbol: symbol.to_string(),
        name: symbol.to_string(),
        isin_code: String::new(),
        listing_date: String::new(),
        industry: industry.to_string(),
        cfi_code: String::new(),
        market: crate::core::declare::StockExchangeMarket::Listed,
    };
    let tdr = IsinAclMapper::from_isin(&isin("9103", "存託憑證")).expect("TDR 應轉換");
    assert_eq!(tdr.industry_id, 39);
    let preferred = IsinAclMapper::from_isin(&isin("79979B", "未分類")).expect("特別股應轉換");
    assert_eq!(preferred.industry_id, 99);
}

/// 公開發行不在市場別對照表內，必須回傳 None 而不是 panic。
#[test]
fn test_to_registration_command_skips_unknown_market() {
    let isin = InternationalSecuritiesIdentificationNumber {
        stock_symbol: "1111".to_string(),
        name: "欣欣水泥".to_string(),
        isin_code: "TW0001111003".to_string(),
        listing_date: "1982/11/24".to_string(),
        industry: "水泥工業".to_string(),
        cfi_code: "ESVUFR".to_string(),
        market: crate::core::declare::StockExchangeMarket::Public,
    };

    assert!(IsinAclMapper::from_isin(&isin).is_none());
}

#[test]
fn test_to_delisted_command_success() {
    let dto = SuspendListing {
        delisting_date: "1120520".to_string(),
        name: "測試下市".to_string(),
        stock_symbol: "9999".to_string(),
    };

    let cmd = DelistedCompanyAclMapper::from_suspend_listing(&dto);
    assert!(cmd.is_some());
    assert_eq!(cmd.unwrap().symbol, "9999");
}

/// 驗證下市日期含多位元組字元時回傳 None 而不是 panic。
///
/// 「éé」是 4 bytes，舊版檢查 len() >= 3 後就做 `[..3]` 切片，
/// byte 3 落在第二個字元中間會直接 panic；改用 .get(..3) 後安全回傳 None。
#[test]
fn test_to_delisted_command_rejects_multibyte_date() {
    let dto = SuspendListing {
        delisting_date: "éé".to_string(), // 4 bytes，切 ..3 會跨字元
        name: "壞資料".to_string(),
        stock_symbol: "9998".to_string(),
    };

    assert!(DelistedCompanyAclMapper::from_suspend_listing(&dto).is_none());

    // 長度不足與非數字內容同樣回傳 None。
    let dto = SuspendListing {
        delisting_date: "台".to_string(), // 3 bytes 但非數字
        name: "壞資料".to_string(),
        stock_symbol: "9997".to_string(),
    };
    assert!(DelistedCompanyAclMapper::from_suspend_listing(&dto).is_none());
}

#[test]
fn test_to_delisted_command_too_old() {
    let dto = SuspendListing {
        delisting_date: "1091231".to_string(),
        name: "測試老舊下市".to_string(),
        stock_symbol: "9999".to_string(),
    };

    let cmd = DelistedCompanyAclMapper::from_suspend_listing(&dto);
    assert!(cmd.is_none(), "應過濾掉民國 110 年之前的下市資料");
}

#[test]
fn test_to_delisted_command_invalid_date() {
    let dto = SuspendListing {
        delisting_date: "12".to_string(),
        name: "無效日期".to_string(),
        stock_symbol: "9999".to_string(),
    };

    let cmd = DelistedCompanyAclMapper::from_suspend_listing(&dto);
    assert!(cmd.is_none(), "日期太短應過濾");
}

#[test]
fn test_etf_to_registration_command() {
    let etf = EtfInfo {
        stock_symbol: "0050".to_string(),
        name: "元大台灣50".to_string(),
        listing_date: "2003/06/30".to_string(),
        industry: "ETF".to_string(),
        market: crate::core::declare::StockExchangeMarket::Listed,
        industry_id: 9001,
    };

    let cmd = EtfAclMapper::from_etf(&etf);
    assert_eq!(cmd.symbol, "0050");
    assert_eq!(cmd.name, "元大台灣50");
    assert_eq!(cmd.market_id, 2);
    assert_eq!(cmd.industry_id, 9001);
}
