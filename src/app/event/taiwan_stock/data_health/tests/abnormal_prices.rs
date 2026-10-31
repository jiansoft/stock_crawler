//! 本週異常價格：只讀期間內的 info 檔（含輪轉檔），依站點加總收盤統計行。

use super::*;
use crate::app::event::taiwan_stock::data_health::abnormal_prices::{
    abnormal_price_line, weekly_abnormal_prices,
};

/// 測試專用的暫存目錄，結束時刪除。
struct LogDir(std::path::PathBuf);

impl LogDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("stock_crawler_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建立暫存目錄");
        Self(dir)
    }

    fn write(&self, file: &str, lines: &[&str]) {
        let body: Vec<String> = lines
            .iter()
            .map(|line| format!("2026-10-08 15:00:10.97 {line}"))
            .collect();
        std::fs::write(self.0.join(file), body.join("\n")).expect("寫入日誌");
    }
}

impl Drop for LogDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 同一天多行（收盤重跑）次數相加、天數算一次；期間外、非 info 檔不算；其他行略過。
#[test]
fn weekly_abnormal_prices_sums_sites_by_day() {
    let dir = LogDir::new("weekly_abnormal_prices");
    dir.write(
        "2026-10-08_default_info.log",
        &[
            "今日過濾異常價格：HiStock 3 次（2 檔：3189、3481）；CnYes 1 次（1 檔：2330）",
            "HiStock 收盤統計：rounds=2262",
        ],
    );
    dir.write(
        "2026-10-08_default_info.15-30-00.log",
        &["今日過濾異常價格：HiStock 2 次（1 檔：6134）"],
    );
    dir.write(
        "2026-10-09_default_info.log",
        &["今日過濾異常價格：HiStock 50 次（12 檔：1101、1102…）"],
    );
    dir.write(
        "2026-10-03_default_info.log",
        &["今日過濾異常價格：Yahoo 9 次（1 檔：2330）"],
    );
    dir.write(
        "2026-10-08_default_warn.log",
        &["今日過濾異常價格：Yahoo 9 次（1 檔：2330）"],
    );

    let weekly = weekly_abnormal_prices(&dir.0, day(4), day(10));
    assert_eq!(
        abnormal_price_line(&weekly),
        "本週異常價格：CnYes 1 次（1 天）、HiStock 55 次（2 天）；⚠️ HiStock 一週超過 50 次，檢查來源"
    );
    let missing = weekly_abnormal_prices(&dir.0.join("missing"), day(4), day(10));
    assert_eq!(abnormal_price_line(&missing), "本週異常價格 0 次");
}
