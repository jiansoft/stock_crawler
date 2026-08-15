$ErrorActionPreference = 'Stop'

# armv7-unknown-linux-musleabihf: Raspberry Pi 3 (armv7l 32-bit, e.g. 5.10.103-v7+) 用的靜態連結執行檔
$Targets = @('aarch64-unknown-linux-musl', 'armv7-unknown-linux-musleabihf')
$BuildProfile = 'release'
$BinName = 'stock_crawler'
$BuildCount = 0
$TotalElapsed = [TimeSpan]::Zero

function Format-Elapsed {
    param(
        [Parameter(Mandatory = $true)]
        [TimeSpan]$Elapsed
    )

    '{0:00}:{1:00}:{2:00}.{3:00}' -f $Elapsed.Hours, $Elapsed.Minutes, $Elapsed.Seconds, [int]($Elapsed.Milliseconds / 10)
}

function Get-CommandOutput {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,
        [string[]]$ArgumentList = @()
    )

    $result = & $FilePath @ArgumentList 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw ($result | Out-String).Trim()
    }

    @($result)
}

function Test-CommandExists {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CommandName
    )

    $null -ne (Get-Command $CommandName -ErrorAction SilentlyContinue)
}

Write-Host '[1/9] Checking Zig...'
if (-not (Test-CommandExists 'zig')) {
    Write-Host 'Zig is not installed or not in PATH.'
    Write-Host 'Please install Zig first: https://ziglang.org/download/'
    exit 1
}

Write-Host '[2/9] Checking CMake...'
if (-not (Test-CommandExists 'cmake')) {
    Write-Host 'CMake is not installed or not in PATH.'
    Write-Host 'Please install CMake first: https://cmake.org/download/'
    exit 1
}



Write-Host '[3/9] Updating Rust toolchain...'
if (-not (Test-CommandExists 'rustup')) {
    Write-Host 'rustup is not installed or not in PATH.'
    Write-Host 'Please install rustup first: https://rustup.rs/'
    exit 1
}
& rustup update
if ($LASTEXITCODE -ne 0) {
    Write-Host 'Failed to update Rust toolchain.'
    exit 1
}

Write-Host '[4/9] Tool versions:'
Get-CommandOutput 'cmake' @('--version') |
    Where-Object { $_ -like 'cmake version*' } |
    ForEach-Object { Write-Host "  - $_" }
Get-CommandOutput 'zig' @('version') | ForEach-Object { Write-Host "  - zig $_" }
Get-CommandOutput 'cargo' @('--version') | ForEach-Object { Write-Host "  - $_" }
try {
    Get-CommandOutput 'cargo-zigbuild' @('--version') | ForEach-Object { Write-Host "  - $_" }
} catch {
    Write-Host '  - cargo-zigbuild not found'
}
Get-CommandOutput 'rustc' @('--version') | ForEach-Object { Write-Host "  - $_" }
if (Test-CommandExists 'rustup') {
    $rustupVersion = cmd /c "rustup --version 2>nul"
    if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($rustupVersion)) {
        Write-Host "  - $rustupVersion"
    } else {
        Write-Host '  - rustup not found'
    }
} else {
    Write-Host '  - rustup not found'
}

Write-Host '[5/9] Ensuring Rust targets...'
foreach ($target in $Targets) {
    Write-Host "  - Adding target $target"
    & rustup target add $target
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Failed to add Rust target: $target"
        exit 1
    }
}

Write-Host '[6/9] Checking cargo-zigbuild...'
& cargo zigbuild -h *> $null
if ($LASTEXITCODE -ne 0) {
    Write-Host 'cargo-zigbuild not found, installing...'
    & cargo install --locked cargo-zigbuild
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'Failed to install cargo-zigbuild.'
        exit 1
    }
}

Write-Host "[7/9] Building $BinName for cross targets..."
foreach ($target in $Targets) {
    $BuildCount += 1
    Write-Host ''
    Write-Host "===== Build $BuildCount`: $target ====="

    $start = Get-Date
    & cargo zigbuild --target $target "--$BuildProfile"
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Build failed for $target."
        Write-Host ''
        Write-Host 'Check these first:'
        Write-Host '  - cmake --version'
        Write-Host '  - zig version'
        Write-Host ''
        exit 1
    }

    $elapsed = (Get-Date) - $start
    $TotalElapsed = $TotalElapsed.Add($elapsed)

    $archSuffix = switch ($target) {
        'aarch64-unknown-linux-musl' { '_arm64' }
        'armv7-unknown-linux-musleabihf' { '_armv7' }
        default { '' }
    }

    $outPath = Join-Path 'target' "$target\$BuildProfile\$BinName"
    if (Test-Path -LiteralPath $outPath) {
        if ($archSuffix) {
            # Rename-Item 在目標已存在時會失敗（「當檔案已存在時，無法建立該檔案」），
            # 於是第二次之後的每一次建置都只把新執行檔留在 $BinName，加後綴的那個
            # 仍停在第一次建置的版本。改用 Move-Item -Force 覆寫。
            $renamedPath = Join-Path 'target' "$target\$BuildProfile\$BinName$archSuffix"
            Move-Item -LiteralPath $outPath -Destination $renamedPath -Force
            $outPath = $renamedPath
        }
        Write-Host "Output binary: $outPath"
    } else {
        Write-Host "Build command finished, but binary not found at: $outPath"
        exit 1
    }

    Write-Host "Elapsed for $target`: $(Format-Elapsed -Elapsed $elapsed)"
}

Write-Host ''
Write-Host "[8/9] Building $BinName for native Windows..."
$winStart = Get-Date
& cargo build "--$BuildProfile"
if ($LASTEXITCODE -ne 0) {
    Write-Host 'Build failed for native Windows target.'
    exit 1
}
$winElapsed = (Get-Date) - $winStart
$TotalElapsed = $TotalElapsed.Add($winElapsed)
$BuildCount += 1

$winOutPath = Join-Path 'target' "$BuildProfile\$BinName.exe"
if (Test-Path -LiteralPath $winOutPath) {
    Write-Host "Output binary: $winOutPath"
} else {
    Write-Host "Build command finished, but binary not found at: $winOutPath"
    exit 1
}
Write-Host "Elapsed for native Windows: $(Format-Elapsed -Elapsed $winElapsed)"

Write-Host ''
Write-Host '[9/9] Done.'
Write-Host "Targets built: $BuildCount"
Write-Host "Total build time: $(Format-Elapsed -Elapsed $TotalElapsed)"
