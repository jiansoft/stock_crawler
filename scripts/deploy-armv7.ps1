<#
.SYNOPSIS
    把 build.ps1 / build.bat 產出的 ARMv7 執行檔部署到 Raspberry Pi 3。

.DESCRIPTION
    對應手動流程：scp 到 /tmp → control.sh update → 驗證。

      1. 本機預檢：直接讀 ELF header 確認這是 32-bit little-endian ARM、
         hard-float（EF_ARM_ABI_FLOAT_HARD）、且沒有 PT_INTERP（musl 靜態連結）。
         這一步是刻意加的：target\ 底下同時躺著 arm64 與 Windows 的產物，
         build.sh 產出的 gnu target 也是動態連結，弄錯了要到 Pi 上才會發現。
      2. 遠端預檢：uname -m 必須是 armv7l，且 <RemoteBase>/control.sh 存在。
      3. scp 執行檔到 /tmp/stock_crawler_armv7（control.sh 的 move 用 uname -m
         反推檔名去 /tmp 找，名字不對只會安靜地跳過搬移然後啟動舊版），
         上傳後比對 sha256 才往下走。
      4. ssh 執行 control.sh update（stop → move → start）。
      5. 驗證：9001（gRPC）／9002（Data API）是否監聽、/api/v1/healthz 是否 200、
         今日 error log 與 nohup.out 的尾巴。healthz 會重試——Pi 3 剛啟動時
         第一個請求特別慢，單發逾時會誤判成失敗。

    這支腳本預設不建置。要連建置一起做，加 -Build（只建 armv7，不會順便建
    arm64 與 Windows；三個 target 都要請直接跑 build.ps1）。

    不同步 .env 與 app.json：Pi 上那兩份是手動維護的正式設定，與 repo 內的
    開發用版本刻意不同，蓋掉會直接把正式站的資料庫／憑證設定弄壞。

    回滾：control.sh 的 move 會把舊執行檔備份成 stock_crawler_armv7.<時間戳>
    並 chmod -x，改名回去、補回執行權限再 ./control.sh restart 即可
    （驗證失敗時腳本會把完整指令印出來）。

.PARAMETER Target
    SSH 目標，預設 pi@192.168.111.138。

.PARAMETER IdentityFile
    SSH 私鑰路徑，預設 $env:USERPROFILE\.ssh\138.key。

.PARAMETER SshPort
    SSH 連接埠，預設 22。

.PARAMETER Binary
    要部署的執行檔。預設先找 target\armv7-unknown-linux-musleabihf\release\stock_crawler_armv7，
    找不到再退回專案根目錄的 stock_crawler_armv7（control.sh docker_build 用的那份）。

.PARAMETER RemoteBase
    Pi 上的部署目錄，預設 /opt/stock_crawler。

.PARAMETER Build
    部署前先用 cargo zigbuild 建置 armv7-unknown-linux-musleabihf release。

.PARAMETER StageOnly
    只上傳並比對 sha256，不執行 control.sh update（正式站不會被重啟）。
    用來驗證連線與檔案，或想自己挑時間再上線時使用。

.PARAMETER SkipVerify
    略過部署後的驗證（仍會重啟服務）。

.EXAMPLE
    .\scripts\deploy-armv7.ps1

.EXAMPLE
    .\scripts\deploy-armv7.ps1 -Build

.EXAMPLE
    .\scripts\deploy-armv7.ps1 -StageOnly
#>
[CmdletBinding()]
param(
    [string]$Target = 'pi@192.168.111.138',
    [string]$IdentityFile = "$env:USERPROFILE\.ssh\138.key",
    [int]$SshPort = 22,
    [string]$Binary,
    [string]$RemoteBase = '/opt/stock_crawler',
    [switch]$Build,
    [switch]$StageOnly,
    [switch]$SkipVerify
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$RustTarget = 'armv7-unknown-linux-musleabihf'
$BinaryName = 'stock_crawler_armv7'
# app.json 的 system.grpc_use_port 與 .env 的 MANUAL_BACKFILL_WEB_ADDR。
$GrpcPort = 9001
$WebPort = 9002

function Invoke-Checked
{
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments
    )

    & $Command @Arguments
    if ($LASTEXITCODE -ne 0)
    {
        throw "指令失敗（exit $LASTEXITCODE）：$Command $( $Arguments -join ' ' )"
    }
}

# 只讀檔頭，不把 29MB 全部載進記憶體。ELF32 的 program header 一定落在前 4KB 內。
function Test-Armv7StaticElf
{
    param([Parameter(Mandatory = $true)][string]$Path)

    $head = New-Object byte[] 4096
    $stream = [IO.File]::OpenRead($Path)
    try
    {
        $read = $stream.Read($head, 0, $head.Length)
    }
    finally
    {
        $stream.Dispose()
    }
    if ($read -lt 64)
    {
        throw "檔案太小，不像是 ELF 執行檔：$Path"
    }

    if ($head[0] -ne 0x7F -or $head[1] -ne 0x45 -or $head[2] -ne 0x4C -or $head[3] -ne 0x46)
    {
        throw "不是 ELF 檔：$Path"
    }
    if ($head[4] -ne 1)
    {
        throw "這是 64-bit ELF（EI_CLASS=$( $head[4] )），Raspberry Pi 3 跑的是 32-bit armv7l。是不是拿到 arm64 那份？"
    }
    if ($head[5] -ne 1)
    {
        throw "不是 little-endian ELF：$Path"
    }

    $machine = [BitConverter]::ToUInt16($head, 0x12)
    if ($machine -ne 0x28)
    {
        throw ("e_machine=0x{0:X} 不是 ARM(0x28)：$Path" -f $machine)
    }

    # ARM 的 e_flags：0x400 = EF_ARM_ABI_FLOAT_HARD。musleabihf 一定有，
    # soft-float 的產物放上去會在啟動時直接掛掉。
    $flags = [BitConverter]::ToUInt32($head, 0x24)
    if (($flags -band 0x400) -eq 0)
    {
        throw ("e_flags=0x{0:X8} 沒有 hard-float 位元，這不是 $RustTarget 的產物" -f $flags)
    }

    # 掃 program header 找 PT_INTERP(3)。有 interpreter 就代表是動態連結
    # （例如 build.sh 的 gnu target），Pi 上不保證有對應的 loader 與 glibc 版本。
    $phoff = [BitConverter]::ToUInt32($head, 0x1C)
    $phentsize = [BitConverter]::ToUInt16($head, 0x2A)
    $phnum = [BitConverter]::ToUInt16($head, 0x2C)
    for ($i = 0; $i -lt $phnum; $i++)
    {
        $offset = $phoff + $i * $phentsize
        if ($offset + 4 -gt $read)
        {
            break
        }
        if ([BitConverter]::ToUInt32($head, $offset) -eq 3)
        {
            throw "這是動態連結的執行檔（含 PT_INTERP），請改用 $RustTarget 的 musl 靜態產物"
        }
    }

    [pscustomobject]@{
        Flags = ('0x{0:X8}' -f $flags)
        SizeMB = [Math]::Round((Get-Item -LiteralPath $Path).Length / 1MB, 1)
    }
}

# ssh/scp 共用的參數：BatchMode 讓沒有金鑰時直接失敗，而不是卡在互動式密碼提示。
$SshArgs = @('-p', "$SshPort", '-o', 'BatchMode=yes')
$ScpArgs = @('-P', "$SshPort", '-o', 'BatchMode=yes')
if (-not [string]::IsNullOrWhiteSpace($IdentityFile))
{
    if (-not (Test-Path -LiteralPath $IdentityFile))
    {
        throw "找不到 SSH 私鑰：$IdentityFile"
    }
    $SshArgs = @('-i', $IdentityFile) + $SshArgs
    $ScpArgs = @('-i', $IdentityFile) + $ScpArgs
}

function Invoke-Remote
{
    param([Parameter(Mandatory = $true)][string]$Script)
    & ssh @SshArgs $Target $Script
}

# --- 0. 建置（-Build）--------------------------------------------------------
if ($Build)
{
    Write-Host "建置 $RustTarget ..." -ForegroundColor Cyan
    foreach ($tool in @('zig', 'cargo'))
    {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue))
        {
            throw "找不到 $tool，請先安裝（完整的工具鏈檢查在 build.ps1）"
        }
    }
    & cargo zigbuild -h *> $null
    if ($LASTEXITCODE -ne 0)
    {
        throw '找不到 cargo-zigbuild，請先執行 build.ps1（會自動安裝）或 cargo install --locked cargo-zigbuild'
    }

    Push-Location $RepoRoot
    try
    {
        Invoke-Checked cargo zigbuild --target $RustTarget --release
    }
    finally
    {
        Pop-Location
    }

    # 跟 build.ps1 一樣加架構後綴：control.sh 的 move 只認 stock_crawler_armv7 這個名字。
    $built = Join-Path $RepoRoot "target\$RustTarget\release\stock_crawler"
    if (-not (Test-Path -LiteralPath $built))
    {
        throw "建置結束但找不到產物：$built"
    }
    Move-Item -LiteralPath $built -Destination (Join-Path $RepoRoot "target\$RustTarget\release\$BinaryName") -Force
    Write-Host ''
}

# --- 1. 本機預檢 -------------------------------------------------------------
if ([string]::IsNullOrWhiteSpace($Binary))
{
    $candidates = @(
    (Join-Path $RepoRoot "target\$RustTarget\release\$BinaryName"),
    (Join-Path $RepoRoot $BinaryName)
    )
    $Binary = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($Binary))
    {
        throw "找不到 $BinaryName（先跑 build.ps1 / build.bat，或用 -Build）。已找過：`n  $( $candidates -join "`n  " )"
    }
}
if (-not (Test-Path -LiteralPath $Binary))
{
    throw "找不到執行檔：$Binary"
}
# 轉成絕對路徑再往下走。相對路徑對 .NET 的 File API 與 scp 這種原生程式來說，
# 基準是行程啟動時的目錄，不是 PowerShell 的目前位置，兩者不一致會讀到別的檔案。
$Binary = (Resolve-Path -LiteralPath $Binary).ProviderPath

Write-Host '本機預檢...' -ForegroundColor Cyan
$elf = Test-Armv7StaticElf -Path $Binary
$localHash = (Get-FileHash -LiteralPath $Binary -Algorithm SHA256).Hash.ToLower()
Write-Host "  BINARY   $Binary ($( $elf.SizeMB ) MB)"
Write-Host "  TARGET   linux/arm v7 hard-float，靜態連結（e_flags $( $elf.Flags )）"
Write-Host "  SHA256   $localHash"

# --- 2. 遠端預檢 -------------------------------------------------------------
$remoteInfo = Invoke-Remote "uname -m; test -f $RemoteBase/control.sh && echo CONTROL_OK || echo CONTROL_MISSING"
if ($LASTEXITCODE -ne 0)
{
    throw "SSH 連線失敗：$Target"
}
$arch = ($remoteInfo | Select-Object -First 1).Trim()
if ($arch -ne 'armv7l')
{
    throw "遠端架構是 $arch，不是 armv7l"
}
if ($remoteInfo -notcontains 'CONTROL_OK')
{
    throw "遠端找不到 $RemoteBase/control.sh"
}
Write-Host "  REMOTE   $Target ($arch, $RemoteBase)"
Write-Host ''

# --- 3. 上傳 ----------------------------------------------------------------
# 固定送到 /tmp，不直接覆蓋執行中的檔案，否則會得到 Text file busy(ETXTBSY)。
Write-Host '上傳執行檔...' -ForegroundColor Cyan
Invoke-Checked scp @ScpArgs $Binary "${Target}:/tmp/$BinaryName"

$remoteHash = (Invoke-Remote "sha256sum /tmp/$BinaryName | cut -d' ' -f1" | Select-Object -First 1).Trim()
if ($remoteHash -ne $localHash)
{
    throw "上傳後 sha256 不符（遠端 $remoteHash），檔案可能在傳輸中損壞，未執行 update"
}
Write-Host "  已上傳並通過 sha256 比對：${Target}:/tmp/$BinaryName"
Write-Host ''

if ($StageOnly)
{
    Write-Host '已上傳但未部署（-StageOnly）。要上線請執行：' -ForegroundColor Yellow
    Write-Host "  ssh $Target `"cd $RemoteBase && ./control.sh update`"" -ForegroundColor Yellow
    return
}

# --- 4. 部署 ----------------------------------------------------------------
Write-Host 'control.sh update...' -ForegroundColor Cyan
Invoke-Remote "cd $RemoteBase && ./control.sh update"
if ($LASTEXITCODE -ne 0)
{
    throw 'control.sh update 失敗'
}
Write-Host ''

if ($SkipVerify)
{
    Write-Host '已略過驗證（-SkipVerify）。' -ForegroundColor Yellow
    return
}

# --- 5. 驗證 ----------------------------------------------------------------
Write-Host '驗證中（等待服務起來）...' -ForegroundColor Cyan
Start-Sleep -Seconds 8

# healthz 重試：Pi 3 上第一個請求特別慢，control.sh start 只保證程序被拉起來。
$verify = @"
echo "--- proc ---"
# 用 control.sh 寫的 pid file 對照，而不是 pgrep：pgrep -f 會連這段 ssh 指令本身
# 一起匹配到，永遠都「找得到程序」。etimes 是啟動至今的秒數，可看出真的重啟過。
pid="`$(cat "$RemoteBase/bin/stock_crawler.pid" 2>/dev/null)"
ps -p "`${pid:-0}" -o pid=,etimes=,args= 2>/dev/null || echo NONE
echo "--- listen ---"
ss -lntp 2>/dev/null | grep -E ':$GrpcPort|:$WebPort' || echo NONE
echo "--- health ---"
for i in 1 2 3 4 5 6; do
  code="`$(curl -s -m 10 -o /dev/null -w '%{http_code}' http://127.0.0.1:$WebPort/api/v1/healthz 2>/dev/null || echo 000)"
  echo "healthz attempt `$i http=`$code"
  if [ "`$code" = "200" ]; then break; fi
  sleep 10
done
echo "--- error log ---"
tail -n 5 "$RemoteBase/log/`$(date +%Y-%m-%d)_default_error.log" 2>/dev/null || echo NONE
echo "--- stdout ---"
tail -n 5 "$RemoteBase/bin/nohup.out" 2>/dev/null || echo NONE
"@
$result = Invoke-Remote $verify
$result | ForEach-Object { Write-Host "  $_" }

$text = $result -join "`n"
$problems = @()
if ($text -notmatch [regex]::Escape("$RemoteBase/$BinaryName"))
{
    $problems += '找不到執行中的程序'
}
if ($text -notmatch ":$GrpcPort")
{
    $problems += "gRPC $GrpcPort 沒有監聽"
}
if ($text -notmatch ":$WebPort")
{
    $problems += "Data API $WebPort 沒有監聽"
}
if ($text -notmatch 'http=200')
{
    $problems += '/api/v1/healthz 沒有回 200'
}

Write-Host ''
if ($problems.Count -gt 0)
{
    Write-Host '部署完成但驗證有問題：' -ForegroundColor Red
    $problems | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    Write-Host '回滾指令：' -ForegroundColor Yellow
    Write-Host "  ssh $Target `"cd $RemoteBase && ls -t $BinaryName.* | head -1`"" -ForegroundColor Yellow
    Write-Host "  ssh $Target `"cd $RemoteBase && ./control.sh stop && mv <上一行的備份檔> $BinaryName && chmod +x $BinaryName && ./control.sh start`"" -ForegroundColor Yellow
    exit 1
}

Write-Host "部署成功：$Target`:$RemoteBase/$BinaryName" -ForegroundColor Green
