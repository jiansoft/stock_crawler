use rand::Rng;

const FIREFOX_VERSIONS: [&str; 80] = [
    "133.0", "132.0", "131.0", "130.0", "129.0", "128.0", "127.0", "126.0", "125.0", "124.0",
    "123.0", "122.0", "121.0", "120.0", "119.0", "118.0", "117.0", "116.0", "115.0", "114.0",
    "113.0", "112.0", "111.0", "110.0", "109.0", "108.0", "107.0", "106.0", "105.0", "104.0",
    "103.0", "102.0", "101.0", "100.0", "99.0", "98.0", "97.0", "96.0", "95.0", "94.0",
    "93.0", "92.0", "91.0", "90.0", "89.0", "88.0", "87.0", "86.0", "85.0", "84.0",
    "83.0", "82.0", "81.0", "80.0", "79.0", "78.0", "77.0", "76.0", "75.0", "74.0",
    "73.0", "72.0", "71.0", "70.0", "69.0", "68.0", "67.0", "66.0", "65.0", "64.0",
    "63.0", "62.0", "61.0", "60.0", "59.0", "58.0", "57.0", "56.0", "55.0", "54.0",
];

const CHROME_VERSIONS: [&str; 100] = [
    "133.0.6943.50", "133.0.6943.60", "133.0.6943.88", "132.0.6834.83", "132.0.6834.110",
    "131.0.6778.85", "131.0.6778.108", "130.0.6723.92", "130.0.6723.117", "129.0.6668.70",
    "129.0.6668.89", "128.0.6613.120", "128.0.6613.138", "127.0.6533.88", "127.0.6533.119",
    "126.0.6478.126", "126.0.6478.182", "125.0.6422.141", "125.0.6422.176", "124.0.6367.201",
    "124.0.6367.243", "123.0.6312.122", "123.0.6312.86", "122.0.6261.129", "122.0.6261.94",
    "121.0.6167.184", "121.0.6167.139", "120.0.6099.130", "120.0.6099.217", "119.0.6045.105",
    "119.0.6045.159", "118.0.5993.88", "118.0.5993.117", "117.0.5938.132", "117.0.5938.149",
    "116.0.5845.179", "116.0.5845.188", "115.0.5790.171", "115.0.5790.102", "114.0.5735.198",
    "114.0.5735.133", "113.0.5672.126", "113.0.5672.93", "112.0.5615.137", "112.0.5615.49",
    "111.0.5563.146", "111.0.5563.64", "110.0.5481.177", "110.0.5481.100", "109.0.5414.119",
    "109.0.5414.87", "108.0.5359.124", "108.0.5359.94", "107.0.5304.121", "107.0.5304.87",
    "106.0.5249.119", "106.0.5249.61", "105.0.5195.125", "105.0.5195.102", "104.0.5112.101",
    "104.0.5112.79", "103.0.5060.134", "103.0.5060.53", "102.0.5005.115", "102.0.5005.61",
    "101.0.4951.67", "101.0.4951.54", "100.0.4896.127", "100.0.4896.88", "99.0.4844.84",
    "99.0.4844.51", "98.0.4758.102", "98.0.4758.80", "97.0.4692.99", "97.0.4692.71",
    "96.0.4664.110", "96.0.4664.45", "95.0.4638.69", "95.0.4638.54", "94.0.4606.81",
    "94.0.4606.61", "93.0.4577.82", "93.0.4577.63", "92.0.4515.159", "92.0.4515.131",
    "91.0.4472.164", "91.0.4472.124", "90.0.4430.212", "90.0.4430.93", "89.0.4389.128",
    "89.0.4389.90", "88.0.4324.182", "88.0.4324.150", "87.0.4280.141", "87.0.4280.88",
    "86.0.4240.198", "86.0.4240.111", "85.0.4183.121", "85.0.4183.102", "84.0.4147.135",
];

const EDGE_VERSIONS: [&str; 70] = [
    "133.0.3048.56", "133.0.3048.46", "132.0.2957.55", "132.0.2957.63", "131.0.2903.86",
    "131.0.2903.112", "130.0.2849.68", "130.0.2849.80", "129.0.2792.52", "129.0.2792.65",
    "128.0.2739.79", "128.0.2739.90", "127.0.2651.98", "127.0.2651.105", "126.0.2592.87",
    "126.0.2592.102", "125.0.2535.92", "125.0.2535.113", "124.0.2478.80", "124.0.2478.97",
    "123.0.2420.81", "123.0.2420.97", "122.0.2365.92", "122.0.2365.106", "121.0.2277.128",
    "121.0.2277.83", "120.0.2210.144", "120.0.2210.91", "119.0.2151.97", "119.0.2151.58",
    "118.0.2088.76", "118.0.2088.46", "117.0.2045.60", "117.0.2045.47", "116.0.1938.76",
    "116.0.1938.62", "115.0.1901.203", "115.0.1901.188", "114.0.1823.82", "114.0.1823.67",
    "113.0.1774.57", "113.0.1774.50", "112.0.1722.77", "112.0.1722.58", "111.0.1661.62",
    "111.0.1661.54", "110.0.1587.69", "110.0.1587.57", "109.0.1518.78", "109.0.1518.70",
    "108.0.1462.54", "108.0.1462.46", "107.0.1418.62", "107.0.1418.56", "106.0.1370.52",
    "106.0.1370.47", "105.0.1343.53", "105.0.1343.42", "104.0.1293.70", "104.0.1293.63",
    "103.0.1264.77", "103.0.1264.62", "102.0.1245.44", "102.0.1245.41", "101.0.1210.53",
    "101.0.1210.47", "100.0.1185.50", "100.0.1185.44", "99.0.1150.55", "99.0.1150.46",
];

const OPERA_VERSIONS: [&str; 50] = [
    "117.0.5405.88", "116.0.5366.91", "115.0.5322.77", "114.0.5282.102", "113.0.5230.86",
    "112.0.5197.53", "111.0.5168.55", "110.0.5130.66", "109.0.5097.80", "108.0.5067.95",
    "107.0.5045.36", "106.0.4998.70", "105.0.4970.48", "104.0.4944.54", "103.0.4928.26",
    "102.0.4880.78", "101.0.4843.43", "100.0.4815.54", "99.0.4788.77", "98.0.4759.39",
    "97.0.4719.63", "96.0.4693.50", "95.0.4635.46", "94.0.4606.65", "93.0.4585.64",
    "92.0.4561.43", "91.0.4516.77", "90.0.4480.84", "89.0.4447.91", "88.0.4412.74",
    "87.0.4390.45", "86.0.4363.59", "85.0.4341.47", "84.0.4316.21", "83.0.4254.27",
    "82.0.4227.43", "81.0.4196.60", "80.0.4170.63", "79.0.4143.50", "78.0.4093.147",
    "77.0.4054.277", "76.0.4017.177", "75.0.3969.171", "74.0.3911.218", "73.0.3856.396",
    "72.0.3815.400", "71.0.3770.284", "70.0.3728.189", "69.0.3686.57", "68.0.3618.206",
];

const BRAVE_VERSIONS: [&str; 30] = [
    "1.73.104", "1.72.101", "1.71.123", "1.70.126", "1.69.168",
    "1.68.141", "1.67.134", "1.66.118", "1.65.133", "1.64.122",
    "1.63.174", "1.62.165", "1.61.120", "1.60.125", "1.59.124",
    "1.58.137", "1.57.64", "1.56.20", "1.55.18", "1.54.122",
    "1.53.111", "1.52.130", "1.51.118", "1.50.125", "1.49.132",
    "1.48.171", "1.47.186", "1.46.144", "1.45.133", "1.44.112",
];

const VIVALDI_VERSIONS: [&str; 30] = [
    "6.9.3447.51", "6.8.3381.57", "6.7.3329.39", "6.6.3271.57", "6.5.3206.63",
    "6.4.3160.47", "6.3.3138.43", "6.2.3105.58", "6.1.3035.111", "6.0.2979.22",
    "5.9.2936.76", "5.8.2911.60", "5.7.2921.68", "5.6.2867.62", "5.5.2805.50",
    "5.4.2753.40", "5.3.2679.68", "5.2.2623.48", "5.1.2567.66", "5.0.2497.48",
    "4.3.2439.70", "4.2.2406.54", "4.1.2369.21", "4.0.2312.38", "3.8.2259.40",
    "3.7.2218.55", "3.6.2165.40", "3.5.2115.87", "3.4.2066.106", "3.3.2022.47",
];

const OS_STRINGS: [&str; 150] = [
    // Windows (modern versions more likely)
    "Windows NT 10.0; Win64; x64",
    "Windows NT 10.0; Win64; x64",
    "Windows NT 10.0; Win64; x64",
    "Windows NT 10.0; Win64; x64",
    "Windows NT 10.0; Win64; x64",
    "Windows NT 10.0; WOW64",
    "Windows NT 10.0",
    "Windows NT 6.3; Win64; x64",
    "Windows NT 6.2; Win64; x64",
    "Windows NT 6.1; Win64; x64",
    "Windows NT 6.1",

    // macOS (modern versions - Intel)
    "Macintosh; Intel Mac OS X 10_15_7",
    "Macintosh; Intel Mac OS X 11_7_10",
    "Macintosh; Intel Mac OS X 11_7_9",
    "Macintosh; Intel Mac OS X 12_7_6",
    "Macintosh; Intel Mac OS X 12_7_4",
    "Macintosh; Intel Mac OS X 13_6_9",
    "Macintosh; Intel Mac OS X 13_6_7",
    "Macintosh; Intel Mac OS X 13_6_5",
    "Macintosh; Intel Mac OS X 14_7_2",
    "Macintosh; Intel Mac OS X 14_7_1",
    "Macintosh; Intel Mac OS X 14_6_1",
    "Macintosh; Intel Mac OS X 14_5",
    "Macintosh; Intel Mac OS X 15_2_1",
    "Macintosh; Intel Mac OS X 15_2",
    "Macintosh; Intel Mac OS X 15_1_1",
    "Macintosh; Intel Mac OS X 15_1",
    "Macintosh; Intel Mac OS X 15_0",

    // macOS (Apple Silicon)
    "Macintosh; Apple Silicon Mac OS X 13_6_9",
    "Macintosh; Apple Silicon Mac OS X 13_6_7",
    "Macintosh; Apple Silicon Mac OS X 14_7_2",
    "Macintosh; Apple Silicon Mac OS X 14_7_1",
    "Macintosh; Apple Silicon Mac OS X 15_2_1",
    "Macintosh; Apple Silicon Mac OS X 15_2",
    "Macintosh; Apple Silicon Mac OS X 15_1",

    // Linux (various distributions)
    "X11; Linux x86_64",
    "X11; Linux x86_64",
    "X11; Linux x86_64",
    "X11; Ubuntu; Linux x86_64",
    "X11; Ubuntu; Linux x86_64",
    "X11; Ubuntu; Linux x86_64",
    "X11; Ubuntu 24.04; Linux x86_64",
    "X11; Ubuntu 22.04; Linux x86_64",
    "X11; Ubuntu 20.04; Linux x86_64",
    "X11; Fedora; Linux x86_64",
    "X11; Fedora 40; Linux x86_64",
    "X11; Fedora 39; Linux x86_64",
    "X11; Debian; Linux x86_64",
    "X11; Debian 12; Linux x86_64",
    "X11; Arch Linux; Linux x86_64",
    "X11; Manjaro; Linux x86_64",
    "X11; Pop!_OS 22.04; Linux x86_64",
    "X11; Pop!_OS; Linux x86_64",
    "X11; Linux Mint 22; Linux x86_64",
    "X11; Linux Mint; Linux x86_64",
    "X11; Zorin OS 17; Linux x86_64",
    "X11; elementary OS 8; Linux x86_64",
    "X11; openSUSE Tumbleweed; Linux x86_64",
    "X11; openSUSE; Linux x86_64",
    "X11; CentOS; Linux x86_64",
    "X11; Red Hat; Linux x86_64",
    "X11; Gentoo; Linux x86_64",
    "X11; Slackware; Linux x86_64",
    "X11; Kali Linux; Linux x86_64",
    "X11; MX Linux; Linux x86_64",
    "X11; Solus; Linux x86_64",
    "X11; EndeavourOS; Linux x86_64",
    "X11; Garuda Linux; Linux x86_64",
    "X11; Linux Lite; Linux x86_64",
    "X11; Deepin; Linux x86_64",
    "X11; Nobara; Linux x86_64",
    "X11; AlmaLinux; Linux x86_64",
    "X11; Rocky Linux; Linux x86_64",

    // Mobile OS (iOS - iPhone)
    "iPhone; CPU iPhone OS 18_2_1 like Mac OS X",
    "iPhone; CPU iPhone OS 18_2 like Mac OS X",
    "iPhone; CPU iPhone OS 18_1_1 like Mac OS X",
    "iPhone; CPU iPhone OS 18_1 like Mac OS X",
    "iPhone; CPU iPhone OS 18_0_1 like Mac OS X",
    "iPhone; CPU iPhone OS 17_7_2 like Mac OS X",
    "iPhone; CPU iPhone OS 17_7_1 like Mac OS X",
    "iPhone; CPU iPhone OS 17_7 like Mac OS X",
    "iPhone; CPU iPhone OS 17_6_1 like Mac OS X",
    "iPhone; CPU iPhone OS 17_6 like Mac OS X",
    "iPhone; CPU iPhone OS 17_5_1 like Mac OS X",
    "iPhone; CPU iPhone OS 16_7_10 like Mac OS X",
    "iPhone; CPU iPhone OS 16_7_5 like Mac OS X",

    // Mobile OS (iOS - iPad)
    "iPad; CPU OS 18_2_1 like Mac OS X",
    "iPad; CPU OS 18_2 like Mac OS X",
    "iPad; CPU OS 18_1_1 like Mac OS X",
    "iPad; CPU OS 18_1 like Mac OS X",
    "iPad; CPU OS 17_7_2 like Mac OS X",
    "iPad; CPU OS 17_7_1 like Mac OS X",
    "iPad; CPU OS 17_7 like Mac OS X",
    "iPad; CPU OS 16_7_10 like Mac OS X",

    // Mobile OS (Android - Samsung)
    "Linux; Android 15; SM-S928B",                              // Samsung Galaxy S24 Ultra
    "Linux; Android 15; SM-S928U",
    "Linux; Android 15; SM-S926B",                              // Samsung Galaxy S24+
    "Linux; Android 15; SM-S926U",
    "Linux; Android 14; SM-S921B",                              // Samsung Galaxy S24
    "Linux; Android 14; SM-S921U",
    "Linux; Android 14; SM-A546B",                              // Samsung Galaxy A54
    "Linux; Android 14; SM-A536B",                              // Samsung Galaxy A53
    "Linux; Android 14; SM-A346B",                              // Samsung Galaxy A34
    "Linux; Android 13; SM-S918B",                              // Samsung Galaxy S23 Ultra
    "Linux; Android 13; SM-S918U",
    "Linux; Android 13; SM-S916B",                              // Samsung Galaxy S23+
    "Linux; Android 13; SM-S911B",                              // Samsung Galaxy S23
    "Linux; Android 13; SM-G998B",                              // Samsung Galaxy S21 Ultra
    "Linux; Android 12; SM-G991B",                              // Samsung Galaxy S21
    "Linux; Android 14; SAMSUNG SM-F946B",                      // Samsung Galaxy Z Fold 5
    "Linux; Android 14; SAMSUNG SM-F946U",
    "Linux; Android 14; SAMSUNG SM-F731B",                      // Samsung Galaxy Z Flip 5
    "Linux; Android 13; SAMSUNG SM-F936B",                      // Samsung Galaxy Z Fold 4
    "Linux; Android 13; SAMSUNG SM-F721B",                      // Samsung Galaxy Z Flip 4

    // Mobile OS (Android - Google Pixel)
    "Linux; Android 15; Pixel 9 Pro XL",
    "Linux; Android 15; Pixel 9 Pro",
    "Linux; Android 15; Pixel 9",
    "Linux; Android 14; Pixel 8 Pro",
    "Linux; Android 14; Pixel 8",
    "Linux; Android 14; Pixel 8a",
    "Linux; Android 13; Pixel 7 Pro",
    "Linux; Android 13; Pixel 7",
    "Linux; Android 13; Pixel 7a",
    "Linux; Android 12; Pixel 6 Pro",
    "Linux; Android 12; Pixel 6",
    "Linux; Android 12; Pixel 6a",

    // Mobile OS (Android - OnePlus)
    "Linux; Android 15; OnePlus 13",
    "Linux; Android 14; OnePlus 12",
    "Linux; Android 14; OnePlus 12R",
    "Linux; Android 13; OnePlus 11",
    "Linux; Android 13; OnePlus 11R",
    "Linux; Android 13; CPH2449",                               // OnePlus Nord 3
    "Linux; Android 12; LE2123",                                // OnePlus 9 Pro

    // Mobile OS (Android - Xiaomi)
    "Linux; Android 14; 24031PN0DC",                            // Xiaomi 14 Pro
    "Linux; Android 14; 23117PN0CC",                            // Xiaomi 14
    "Linux; Android 13; Xiaomi 2211133C",                       // Xiaomi 13 Pro
    "Linux; Android 13; 2211133G",                              // Xiaomi 13
    "Linux; Android 13; 23028RA60L",                            // Xiaomi Redmi Note 13 Pro
    "Linux; Android 13; M2102J20SG",                            // Xiaomi Mi 11
    "Linux; Android 12; 22041216G",                             // Xiaomi 12

    // Mobile OS (Android - Others)
    "Linux; Android 14; OPPO Find X7 Pro",
    "Linux; Android 14; vivo X100 Pro",
    "Linux; Android 13; Realme GT 5 Pro",
    "Linux; Android 13; Nothing Phone (2)",
    "Linux; Android 12; Motorola Edge 40 Pro",

    // Chrome OS
    "X11; CrOS x86_64 15917.22.0",
    "X11; CrOS x86_64 15823.14.0",
    "X11; CrOS x86_64 15786.82.0",
    "X11; CrOS aarch64 15823.14.0",
    "X11; CrOS aarch64 15786.82.0",
];

fn gen_firefox_ua() -> String {
    let mut rng = rand::rng();
    let version = FIREFOX_VERSIONS[rng.random_range(..FIREFOX_VERSIONS.len())];
    let os = OS_STRINGS[rng.random_range(..OS_STRINGS.len())];

    // Handle mobile iOS specifically
    if os.starts_with("iPhone") || os.starts_with("iPad") {
        format!(
            "Mozilla/5.0 ({}; rv:{}) Gecko/20100101 Firefox/{}",
            os, version, version
        )
    } else {
        format!(
            "Mozilla/5.0 ({}; rv:{}) Gecko/20100101 Firefox/{}",
            os, version, version
        )
    }
}

fn gen_chrome_ua() -> String {
    let mut rng = rand::rng();
    let version = CHROME_VERSIONS[rng.random_range(0..CHROME_VERSIONS.len())];
    let os = OS_STRINGS[rng.random_range(0..OS_STRINGS.len())];

    // Handle mobile platforms
    if os.starts_with("iPhone") || os.starts_with("iPad") {
        format!(
            "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) CriOS/{} Mobile/15E148 Safari/604.1",
            os, version.split('.').next().unwrap_or("133")
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

pub fn gen_random_ua() -> String {
    let mut rng = rand::rng();
    let choice = rng.random_range(0..20);
    match choice {
        0..=7 => gen_chrome_ua(),         // 40% Chrome (most popular)
        8..=10 => gen_firefox_ua(),       // 15% Firefox
        11..=12 => gen_edge_ua(),         // 10% Edge
        13 => gen_safari_desktop_ua(),    // 5% Safari Desktop
        14 => gen_safari_mobile_ua(),     // 5% Safari Mobile
        15 => gen_samsung_internet_ua(),  // 5% Samsung Internet
        16 => gen_opera_ua(),             // 5% Opera
        17 => gen_brave_ua(),             // 5% Brave
        18 => gen_vivaldi_ua(),           // 5% Vivaldi
        _ => gen_chrome_ua(),             // 5% Fallback to Chrome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gen_random_ua_variety() {
        // Generate 20 user agents to test variety
        println!("\n=== Testing User Agent Variety ===");
        for i in 1..=20 {
            let ua = gen_random_ua();
            println!("{:2}. {}", i, ua);
        }
    }

    #[test]
    fn test_all_browser_generators() {
        println!("\n=== Testing All Browser Generators ===");

        println!("\nChrome:");
        println!("  {}", gen_chrome_ua());

        println!("\nFirefox:");
        println!("  {}", gen_firefox_ua());

        println!("\nEdge:");
        println!("  {}", gen_edge_ua());

        println!("\nOpera:");
        println!("  {}", gen_opera_ua());

        println!("\nBrave:");
        println!("  {}", gen_brave_ua());

        println!("\nVivaldi:");
        println!("  {}", gen_vivaldi_ua());

        println!("\nSafari Desktop:");
        println!("  {}", gen_safari_desktop_ua());

        println!("\nSafari Mobile:");
        println!("  {}", gen_safari_mobile_ua());

        println!("\nSamsung Internet:");
        println!("  {}", gen_samsung_internet_ua());
    }

    #[test]
    fn test_ua_formats() {
        // Test that all UAs contain expected patterns
        for _ in 0..100 {
            let ua = gen_random_ua();
            assert!(ua.starts_with("Mozilla/5.0"), "UA should start with Mozilla/5.0: {}", ua);
            assert!(ua.len() > 50, "UA should be reasonably long: {}", ua);
        }
    }
}
