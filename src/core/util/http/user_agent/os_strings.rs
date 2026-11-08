//! User-Agent 用的作業系統字串（Windows／macOS／Linux 等，常見版本重複出現以提高抽中機率）。

pub(super) const OS_STRINGS: [&str; 150] = [
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
    "Linux; Android 15; SM-S928B", // Samsung Galaxy S24 Ultra
    "Linux; Android 15; SM-S928U",
    "Linux; Android 15; SM-S926B", // Samsung Galaxy S24+
    "Linux; Android 15; SM-S926U",
    "Linux; Android 14; SM-S921B", // Samsung Galaxy S24
    "Linux; Android 14; SM-S921U",
    "Linux; Android 14; SM-A546B", // Samsung Galaxy A54
    "Linux; Android 14; SM-A536B", // Samsung Galaxy A53
    "Linux; Android 14; SM-A346B", // Samsung Galaxy A34
    "Linux; Android 13; SM-S918B", // Samsung Galaxy S23 Ultra
    "Linux; Android 13; SM-S918U",
    "Linux; Android 13; SM-S916B",         // Samsung Galaxy S23+
    "Linux; Android 13; SM-S911B",         // Samsung Galaxy S23
    "Linux; Android 13; SM-G998B",         // Samsung Galaxy S21 Ultra
    "Linux; Android 12; SM-G991B",         // Samsung Galaxy S21
    "Linux; Android 14; SAMSUNG SM-F946B", // Samsung Galaxy Z Fold 5
    "Linux; Android 14; SAMSUNG SM-F946U",
    "Linux; Android 14; SAMSUNG SM-F731B", // Samsung Galaxy Z Flip 5
    "Linux; Android 13; SAMSUNG SM-F936B", // Samsung Galaxy Z Fold 4
    "Linux; Android 13; SAMSUNG SM-F721B", // Samsung Galaxy Z Flip 4
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
    "Linux; Android 13; CPH2449", // OnePlus Nord 3
    "Linux; Android 12; LE2123",  // OnePlus 9 Pro
    // Mobile OS (Android - Xiaomi)
    "Linux; Android 14; 24031PN0DC",      // Xiaomi 14 Pro
    "Linux; Android 14; 23117PN0CC",      // Xiaomi 14
    "Linux; Android 13; Xiaomi 2211133C", // Xiaomi 13 Pro
    "Linux; Android 13; 2211133G",        // Xiaomi 13
    "Linux; Android 13; 23028RA60L",      // Xiaomi Redmi Note 13 Pro
    "Linux; Android 13; M2102J20SG",      // Xiaomi Mi 11
    "Linux; Android 12; 22041216G",       // Xiaomi 12
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
