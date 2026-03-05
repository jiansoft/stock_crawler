
@echo off
setlocal

set TARGET=aarch64-unknown-linux-musl
set PROFILE=release
set BIN_NAME=stock_crawler

echo [1/9] Checking Zig...
zig version >nul 2>&1
if errorlevel 1 (
  echo Zig is not installed or not in PATH.
  echo Please install Zig first: https://ziglang.org/download/
  exit /b 1
)

echo [2/9] Checking CMake...
cmake --version >nul 2>&1
if errorlevel 1 (
  echo CMake is not installed or not in PATH.
  echo Please install CMake first: https://cmake.org/download/
  exit /b 1
)

echo [3/9] Checking protoc...
protoc --version >nul 2>&1
if errorlevel 1 (
  echo protoc is not installed or not in PATH.
  echo Install protobuf compiler and ensure protoc is available.
  exit /b 1
)

echo [4/9] Tool versions:
for /f "delims=" %%i in ('protoc --version') do echo   - %%i
for /f "delims=" %%i in ('cmake --version ^| findstr /B /C:"cmake version"') do echo   - %%i
for /f "delims=" %%i in ('zig version') do echo   - zig %%i

echo [5/9] Ensuring Rust target %TARGET%...
rustup target add %TARGET%
if errorlevel 1 (
  echo Failed to add Rust target: %TARGET%
  exit /b 1
)

echo [6/9] Checking cargo-zigbuild...
cargo zigbuild -h >nul 2>&1
if errorlevel 1 (
  echo cargo-zigbuild not found, installing...
  cargo install --locked cargo-zigbuild
  if errorlevel 1 (
    echo Failed to install cargo-zigbuild.
    exit /b 1
  )
)

echo [7/9] Updating dependencies...
cargo update
if errorlevel 1 (
  echo Failed to update dependencies.
  exit /b 1
)

echo [8/9] Building %BIN_NAME% for Alpine ARM64...
cargo zigbuild --target %TARGET% --%PROFILE%
if errorlevel 1 (
  echo Build failed.
  echo.
  echo Check these first:
  echo   - protoc --version
  echo   - cmake --version
  echo   - zig version
  echo.
  exit /b 1
)

set OUT_PATH=target\%TARGET%\%PROFILE%\%BIN_NAME%
echo [9/9] Done.
if exist "%OUT_PATH%" (
  echo Output binary: %OUT_PATH%
) else (
  echo Build command finished, but binary not found at: %OUT_PATH%
  exit /b 1
)

endlocal
