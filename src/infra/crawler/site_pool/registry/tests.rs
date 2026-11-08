//! 站點池清單 (Registry)的測試。

use super::*;

/// 備援站點池只少了 Yahoo，其他站點與順序不變。
#[test]
fn backup_price_sites_exclude_only_yahoo() {
    let names = BACKUP_PRICE_SITES
        .iter()
        .map(|site| site.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["Fugle", "NStock", "CnYes", "BigGo"]);
}
