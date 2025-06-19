use rand::Rng;

const FIREFOX_VERSIONS: [&str; 30] = [
    "98.0", "97.0", "96.0", "95.0", "94.0", "93.0", "92.0", "91.0", "90.0", "89.0", "88.0", "87.0",
    "86.0", "85.0", "84.0", "83.0", "82.0", "81.0", "80.0", "79.0", "78.0", "77.0", "76.0", "75.0",
    "74.0", "73.0", "72.0", "71.0", "70.0", "69.0",
];

const CHROME_VERSIONS: [&str; 38] = [
    "37.0.2062.124",
    "40.0.2214.93",
    "41.0.2228.0",
    "49.0.2623.112",
    "55.0.2883.87",
    "56.0.2924.87",
    "57.0.2987.133",
    "61.0.3163.100",
    "63.0.3239.132",
    "64.0.3282.0",
    "65.0.3325.146",
    "68.0.3440.106",
    "69.0.3497.100",
    "70.0.3538.102",
    "74.0.3729.169",
    "75.0.3829.169",
    "76.0.3929.169",
    "76.0.3929.169",
    "91.0.4472.124",
    "92.0.4515.131",
    "93.0.4577.63",
    "94.0.4606.61",
    "95.0.4638.69",
    "96.0.4664.45",
    "97.0.4692.71",
    "98.0.4758.102",
    "99.0.4844.51",
    "100.0.4896.60",
    "101.0.4951.39",
    "102.0.4987.18",
    "103.0.5012.33",
    "104.0.5058.50",
    "105.0.5093.16",
    "106.0.5126.28",
    "107.0.5160.15",
    "108.0.5196.30",
    "109.0.5221.33",
    "110.0.5253.19",
];

const OPERA_VERSIONS: [&str; 6] = [
    "2.7.62 Version/11.00",
    "2.2.15 Version/10.10",
    "2.9.168 Version/11.50",
    "2.2.15 Version/10.00",
    "2.8.131 Version/11.11",
    "2.5.24 Version/10.54",
];

const OS_STRINGS: [&str; 42] = [
    "Windows NT 10.0",                 // Windows 10
    "Windows NT 6.3",                  // Windows 8.1
    "Windows NT 6.2",                  // Windows 8
    "Windows NT 6.1",                  // Windows 7
    "Windows NT 6.0",                  // Windows Vista
    "Windows NT 5.2",                  // Windows Server 2003; Windows XP x64 Edition
    "Windows NT 5.1",                  // Windows XP
    "Windows NT 5.01",                 // Windows 2000, Service Pack 1 (SP1)
    "Windows NT 5.0",                  // Windows 2000
    "Windows NT 4.0",                  // Microsoft Windows NT 4.0
    "Macintosh; Intel Mac OS X 10_6",  // Mac OS X Snow Leopard
    "Macintosh; Intel Mac OS X 10_7",  // Mac OS X Lion
    "Macintosh; Intel Mac OS X 10_8",  // Mac OS X Mountain Lion
    "Macintosh; Intel Mac OS X 10_9",  // Mac OS X Mavericks
    "Macintosh; Intel Mac OS X 10_10", // Mac OS X Yosemite
    "Macintosh; Intel Mac OS X 10_11", // Mac OS X El Capitan
    "Macintosh; Intel Mac OS X 10_12", // macOS Sierra
    "Macintosh; Intel Mac OS X 10_13", // macOS High Sierra
    "Macintosh; Intel Mac OS X 10_14", // macOS Mojave
    "Macintosh; Intel Mac OS X 10_15", // macOS Catalina
    "Macintosh; Intel Mac OS X 11_0",  // macOS Big Sur
    "Macintosh; Intel Mac OS X 12_0",  // macOS Monterey
    "X11; Ubuntu; Linux x86_64",
    "X11; Linux x86_64",
    "X11; Fedora; Linux x86_64",
    "X11; Debian; Linux x86_64",
    "X11; CentOS; Linux x86_64",
    "X11; openSUSE; Linux x86_64",
    "X11; Arch Linux; Linux x86_64",
    "X11; Gentoo; Linux x86_64",
    "X11; Slackware; Linux x86_64",
    "X11; Mandriva; Linux x86_64",
    "X11; Red Hat; Linux x86_64",
    "X11; Mint; Linux x86_64",
    "X11; Zorin; Linux x86_64",
    "X11; elementary OS; Linux x86_64",
    "X11; Manjaro; Linux x86_64",
    "X11; Pop!_OS; Linux x86_64",
    "X11; Kali Linux; Linux x86_64",
    "X11; Tails; Linux x86_64",
    "X11; MX Linux; Linux x86_64",
    "X11; Solus; Linux x86_64",
];

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
    format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{} Safari/537.36",
        os, version
    )
}

fn gen_opera_ua() -> String {
    let mut rng = rand::rng();
    let version = OPERA_VERSIONS[rng.random_range(0..OPERA_VERSIONS.len())];
    let os = OS_STRINGS[rng.random_range(0..OS_STRINGS.len())];
    format!("Opera/9.80 ({}; U; en) Presto/{}", os, version)
}

pub fn gen_random_ua() -> String {
    let mut rng = rand::rng();
    let choice = rng.random_range(0..3);
    match choice {
        0 => gen_firefox_ua(),
        1 => gen_chrome_ua(),
        _ => gen_opera_ua(),
    }
}
