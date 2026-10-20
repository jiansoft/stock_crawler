//! 隨機產生瀏覽器 User-Agent，降低規律請求被來源站判定為爬蟲的機率。
//!
//! 各瀏覽器的版本號與作業系統字串在 [`versions`]。

mod versions;

use rand::RngExt;

use self::versions::*;

fn gen_firefox_ua() -> String {
    let mut rng = rand::rng();
    let version = FIREFOX_VERSIONS[rng.random_range(..FIREFOX_VERSIONS.len())];
    let os = OS_STRINGS[rng.random_range(..OS_STRINGS.len())];

    format!(
        "Mozilla/5.0 ({}; rv:{}) Gecko/20100101 Firefox/{}",
        os, version, version
    )
}

fn gen_chrome_ua() -> String {
    let mut rng = rand::rng();
    let version = CHROME_VERSIONS[rng.random_range(0..CHROME_VERSIONS.len())];
    let os = OS_STRINGS[rng.random_range(0..OS_STRINGS.len())];

    // Handle mobile platforms
    if os.starts_with("iPhone") || os.starts_with("iPad") {
        format!(
            "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) CriOS/{} Mobile/15E148 Safari/604.1",
            os,
            version.split('.').next().unwrap_or("133")
        )
    } else if os.starts_with("Linux; Android") {
        format!(
            "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{} Mobile Safari/537.36",
            os, version
        )
    } else {
        format!(
            "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{} Safari/537.36",
            os, version
        )
    }
}

fn gen_edge_ua() -> String {
    let mut rng = rand::rng();
    let version = EDGE_VERSIONS[rng.random_range(0..EDGE_VERSIONS.len())];
    // Edge primarily runs on Windows and macOS
    let os_subset = [
        "Windows NT 10.0; Win64; x64",
        "Windows NT 10.0; Win64; x64",
        "Windows NT 10.0; Win64; x64",
        "Macintosh; Intel Mac OS X 10_15_7",
        "Macintosh; Intel Mac OS X 13_6_5",
        "Macintosh; Intel Mac OS X 14_7_1",
        "Macintosh; Intel Mac OS X 15_2",
    ];
    let os = os_subset[rng.random_range(0..os_subset.len())];
    let chrome_ver = version.split('.').next().unwrap_or("133");

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36 Edg/{}",
        os, chrome_ver, version
    )
}

fn gen_opera_ua() -> String {
    let mut rng = rand::rng();
    let version = OPERA_VERSIONS[rng.random_range(0..OPERA_VERSIONS.len())];
    let os = OS_STRINGS[rng.random_range(0..OS_STRINGS.len())];

    // Modern Opera uses Chromium, so use Chromium-based UA
    let chrome_base = version.split('.').collect::<Vec<&str>>();
    let chrome_ver = chrome_base[0];

    // Only desktop platforms for Opera
    if os.starts_with("iPhone") || os.starts_with("iPad") || os.starts_with("Linux; Android") {
        // Fallback to Chrome for mobile
        gen_chrome_ua()
    } else {
        format!(
            "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36 OPR/{}",
            os, chrome_ver, version
        )
    }
}

fn gen_brave_ua() -> String {
    let mut rng = rand::rng();
    let version = BRAVE_VERSIONS[rng.random_range(0..BRAVE_VERSIONS.len())];
    // Brave primarily runs on desktop platforms
    let desktop_os: Vec<&str> = OS_STRINGS
        .iter()
        .filter(|os| {
            os.starts_with("Windows") || os.starts_with("Macintosh") || os.starts_with("X11")
        })
        .copied()
        .collect();
    let os = desktop_os[rng.random_range(0..desktop_os.len())];

    // Brave uses Chromium, map Brave version to approximate Chrome version
    let chrome_ver = version.split('.').nth(1).unwrap_or("130");

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36",
        os, chrome_ver
    )
}

fn gen_vivaldi_ua() -> String {
    let mut rng = rand::rng();
    let version = VIVALDI_VERSIONS[rng.random_range(0..VIVALDI_VERSIONS.len())];
    // Vivaldi primarily runs on desktop platforms
    let desktop_os: Vec<&str> = OS_STRINGS
        .iter()
        .filter(|os| {
            os.starts_with("Windows") || os.starts_with("Macintosh") || os.starts_with("X11")
        })
        .copied()
        .collect();
    let os = desktop_os[rng.random_range(0..desktop_os.len())];

    // Vivaldi uses Chromium
    let chrome_base = version.split('.').next().unwrap_or("6");
    // Map Vivaldi 6.x to Chrome 130+, 5.x to Chrome 120+
    let chrome_ver = match chrome_base {
        "6" => "130",
        "5" => "120",
        "4" => "110",
        _ => "100",
    };

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36 Vivaldi/{}",
        os, chrome_ver, version
    )
}

fn gen_safari_mobile_ua() -> String {
    let mut rng = rand::rng();
    // Select from iOS devices in OS_STRINGS
    let ios_devices: Vec<&str> = OS_STRINGS
        .iter()
        .filter(|os| os.starts_with("iPhone") || os.starts_with("iPad"))
        .copied()
        .collect();
    let os = ios_devices[rng.random_range(0..ios_devices.len())];
    let webkit_versions = ["605.1.15", "604.1", "605.2.3", "606.1.1", "618.1.15"];
    let webkit = webkit_versions[rng.random_range(0..webkit_versions.len())];
    let safari_versions = ["18.2", "18.1", "18.0", "17.7", "17.6"];
    let safari_ver = safari_versions[rng.random_range(0..safari_versions.len())];

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/{} (KHTML, like Gecko) Version/{} Mobile/15E148 Safari/604.1",
        os, webkit, safari_ver
    )
}

fn gen_safari_desktop_ua() -> String {
    let mut rng = rand::rng();
    // Select from macOS in OS_STRINGS
    let macos_systems: Vec<&str> = OS_STRINGS
        .iter()
        .filter(|os| os.starts_with("Macintosh"))
        .copied()
        .collect();
    let os = macos_systems[rng.random_range(0..macos_systems.len())];
    let webkit_versions = ["605.1.15", "604.1", "605.2.3", "618.1.15"];
    let webkit = webkit_versions[rng.random_range(0..webkit_versions.len())];
    let safari_versions = ["18.2", "18.1", "18.0", "17.7", "17.6", "17.5"];
    let safari_ver = safari_versions[rng.random_range(0..safari_versions.len())];

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/{} (KHTML, like Gecko) Version/{} Safari/{}",
        os, webkit, safari_ver, webkit
    )
}

fn gen_samsung_internet_ua() -> String {
    let mut rng = rand::rng();
    // Select from Samsung Android devices in OS_STRINGS
    let samsung_devices: Vec<&str> = OS_STRINGS
        .iter()
        .filter(|os| os.contains("SM-") || os.contains("SAMSUNG"))
        .copied()
        .collect();
    let os = samsung_devices[rng.random_range(0..samsung_devices.len())];
    let samsung_versions = ["26.0.1.1", "25.0.5.3", "24.0.2.1", "23.0.1.5", "22.0.2.3"];
    let samsung_ver = samsung_versions[rng.random_range(0..samsung_versions.len())];

    format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/{} Mobile Safari/537.36",
        os, samsung_ver
    )
}

/// 產生一組隨機瀏覽器 User-Agent。
///
/// 目前會依預設權重從 Chrome、Firefox、Edge、Safari、Samsung Internet、
/// Opera、Brave、Vivaldi 等常見瀏覽器樣板中隨機挑選，降低 crawler 被目標站
/// 以固定 User-Agent 辨識的機率。
pub fn gen_random_ua() -> String {
    let mut rng = rand::rng();
    let choice = rng.random_range(0..20);
    match choice {
        0..=7 => gen_chrome_ua(),        // 40% Chrome (most popular)
        8..=10 => gen_firefox_ua(),      // 15% Firefox
        11..=12 => gen_edge_ua(),        // 10% Edge
        13 => gen_safari_desktop_ua(),   // 5% Safari Desktop
        14 => gen_safari_mobile_ua(),    // 5% Safari Mobile
        15 => gen_samsung_internet_ua(), // 5% Samsung Internet
        16 => gen_opera_ua(),            // 5% Opera
        17 => gen_brave_ua(),            // 5% Brave
        18 => gen_vivaldi_ua(),          // 5% Vivaldi
        _ => gen_chrome_ua(),            // 5% Fallback to Chrome
    }
}

#[cfg(test)]
mod tests;
