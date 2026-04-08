
@echo off
setlocal EnableExtensions EnableDelayedExpansion

set TARGETS=aarch64-unknown-linux-musl
set PROFILE=release
set BIN_NAME=stock_crawler
set BUILD_COUNT=0
set TOTAL_ELAPSED_CS=0

echo [1/8] Checking Zig...
zig version >nul 2>&1
if errorlevel 1 (
  echo Zig is not installed or not in PATH.
  echo Please install Zig first: https://ziglang.org/download/
  exit /b 1
)

echo [2/8] Checking CMake...
cmake --version >nul 2>&1
if errorlevel 1 (
  echo CMake is not installed or not in PATH.
  echo Please install CMake first: https://cmake.org/download/
  exit /b 1
)

echo [3/8] Checking protoc...
protoc --version >nul 2>&1
if errorlevel 1 (
  echo protoc is not installed or not in PATH.
  echo Install protobuf compiler and ensure protoc is available.
  exit /b 1
)

echo [4/8] Tool versions:
for /f "delims=" %%i in ('protoc --version') do echo   - %%i
for /f "delims=" %%i in ('cmake --version ^| findstr /B /C:"cmake version"') do echo   - %%i
for /f "delims=" %%i in ('zig version') do echo   - zig %%i
for /f "delims=" %%i in ('cargo --version') do echo   - %%i
for /f "delims=" %%i in ('rustc --version') do echo   - %%i
rustup --version >nul 2>&1
if errorlevel 1 (
  echo   - rustup not found
) else (
  for /f "delims=" %%i in ('rustup --version 2^>nul') do echo   - %%i
)

echo [5/8] Ensuring Rust targets...
for %%T in (%TARGETS%) do (
  echo   - Adding target %%T
  rustup target add %%T
  if errorlevel 1 (
    echo Failed to add Rust target: %%T
    exit /b 1
  )
)

echo [6/8] Checking cargo-zigbuild...
cargo zigbuild -h >nul 2>&1
if errorlevel 1 (
  echo cargo-zigbuild not found, installing...
  cargo install --locked cargo-zigbuild
  if errorlevel 1 (
    echo Failed to install cargo-zigbuild.
    exit /b 1
  )
)

echo [7/8] Building %BIN_NAME%...
for %%T in (%TARGETS%) do (
  set /a BUILD_COUNT+=1
  echo.
  echo ===== Build !BUILD_COUNT!: %%T =====
  call :GetTimeCentis BUILD_START_CS
  cargo zigbuild --target %%T --%PROFILE%
  if errorlevel 1 (
    echo Build failed for %%T.
    echo.
    echo Check these first:
    echo   - protoc --version
    echo   - cmake --version
    echo   - zig version
    echo.
    exit /b 1
  )

  call :GetTimeCentis BUILD_END_CS
  set /a BUILD_ELAPSED_CS=!BUILD_END_CS! - !BUILD_START_CS!
  if !BUILD_ELAPSED_CS! lss 0 set /a BUILD_ELAPSED_CS+=8640000
  set /a TOTAL_ELAPSED_CS+=BUILD_ELAPSED_CS

  set OUT_PATH=target\%%T\%PROFILE%\%BIN_NAME%
  if exist "!OUT_PATH!" (
    echo Output binary: !OUT_PATH!
  ) else (
    echo Build command finished, but binary not found at: !OUT_PATH!
    exit /b 1
  )

  call :FormatElapsed !BUILD_ELAPSED_CS! BUILD_ELAPSED_TEXT
  echo Elapsed for %%T: !BUILD_ELAPSED_TEXT!
)

call :FormatElapsed %TOTAL_ELAPSED_CS% TOTAL_ELAPSED_TEXT
echo.
echo [8/8] Done.
echo Targets built: %BUILD_COUNT%
echo Total build time: %TOTAL_ELAPSED_TEXT%

endlocal
exit /b 0

:GetTimeCentis
setlocal
set "TIME_RAW=%time: =0%"
for /f "tokens=1-4 delims=:." %%a in ("%TIME_RAW%") do (
  set /a "TIME_CS=(((1%%a-100)*60)+(1%%b-100))*6000 + ((1%%c-100)*100) + (1%%d-100)"
)
endlocal & set "%~1=%TIME_CS%"
exit /b 0

:FormatElapsed
setlocal
set /a "ELAPSED_CS=%~1"
set /a "ELAPSED_TOTAL_SECONDS=ELAPSED_CS / 100"
set /a "ELAPSED_HOURS=ELAPSED_TOTAL_SECONDS / 3600"
set /a "ELAPSED_MINUTES=(ELAPSED_TOTAL_SECONDS %% 3600) / 60"
set /a "ELAPSED_SECONDS=ELAPSED_TOTAL_SECONDS %% 60"
set /a "ELAPSED_CENTIS=ELAPSED_CS %% 100"
if %ELAPSED_HOURS% lss 10 set "ELAPSED_HOURS=0%ELAPSED_HOURS%"
if %ELAPSED_MINUTES% lss 10 set "ELAPSED_MINUTES=0%ELAPSED_MINUTES%"
if %ELAPSED_SECONDS% lss 10 set "ELAPSED_SECONDS=0%ELAPSED_SECONDS%"
if %ELAPSED_CENTIS% lss 10 set "ELAPSED_CENTIS=0%ELAPSED_CENTIS%"
set "ELAPSED_TEXT=%ELAPSED_HOURS%:%ELAPSED_MINUTES%:%ELAPSED_SECONDS%.%ELAPSED_CENTIS%"
endlocal & set "%~2=%ELAPSED_TEXT%"
exit /b 0
