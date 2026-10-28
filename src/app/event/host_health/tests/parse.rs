//! 解析 `get_throttled`、`vcgencmd`、`/proc/meminfo` 與部署標記內容的測試。

use super::*;

#[test]
fn parse_throttled_accepts_bare_and_prefixed_hex() {
    assert_eq!(parse_throttled("0\n"), Some(0));
    assert_eq!(parse_throttled("50005\n"), Some(0x50005));
    assert_eq!(parse_throttled("0x70000"), Some(0x70000));
    assert_eq!(parse_throttled("0X4"), Some(4));
    assert_eq!(parse_throttled("throttled=0x0"), None);
    assert_eq!(parse_throttled(""), None);
}

#[test]
fn parse_vcgencmd_reads_throttled_line() {
    assert_eq!(
        parse_vcgencmd_throttled("throttled=0x70000\n"),
        Some(0x70000)
    );
    assert_eq!(parse_vcgencmd_throttled("throttled=0x0"), Some(0));
    assert_eq!(parse_vcgencmd_throttled("0x70000"), None);
    assert_eq!(parse_vcgencmd_throttled("throttled=zz"), None);
}

#[test]
fn parse_meminfo_reads_available_and_swap() {
    let text = "MemTotal:         945364 kB\nMemFree:           48000 kB\nMemAvailable:     705164 kB\nSwapTotal:        204796 kB\nSwapFree:          42060 kB\n";
    let info = parse_meminfo(text).unwrap();
    assert_eq!(
        info,
        MemInfo {
            available_kib: 705_164,
            swap_total_kib: 204_796,
            swap_free_kib: 42_060,
        }
    );
    let percent = info.swap_used_percent().unwrap();
    assert!((percent - 79.46).abs() < 0.01, "{percent}");

    // 沒有 swap 欄位視為沒有 swap；缺 MemAvailable 無法判斷。
    let no_swap = parse_meminfo("MemAvailable: 1024 kB\n").unwrap();
    assert_eq!(no_swap.swap_used_percent(), None);
    assert_eq!(parse_meminfo("MemTotal: 1024 kB\n"), None);
}

/// 標記內容的 `unit=` 優先，沒有就用檔名；有 `finished=` 才算已結束。
#[test]
fn parse_deploy_marker_reads_unit_and_finished() {
    let age = Duration::from_secs(90);
    assert_eq!(
        parse_deploy_marker(
            "unit=stock_go\nstarted=2026-10-28T08:05:00+08:00\n",
            "file",
            age
        ),
        DeployMarker {
            unit: "stock_go".to_owned(),
            age,
            finished: false,
        }
    );
    let done = parse_deploy_marker(
        "unit=stock-mcp\r\nstarted=x\r\nfinished=2026-10-28T08:10:00+08:00\r\n",
        "file",
        age,
    );
    assert_eq!(done.unit, "stock-mcp");
    assert!(done.finished);
    assert_eq!(
        parse_deploy_marker("unit=\n", "stock_crawler", age).unit,
        "stock_crawler"
    );
    assert_eq!(
        parse_deploy_marker("", "stock_crawler", age).unit,
        "stock_crawler"
    );
}
