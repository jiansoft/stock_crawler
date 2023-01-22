
@ECHO OFF
SET OPENSSL_DIR = 'D:\Project\opensource\vcpkg\installed\x64-windows-static'
SET OPENSSL_INCLUDE_DIR = 'D:\Project\opensource\vcpkg\installed\x64-windows-static\include'
SET OPENSSL_LIB_DIR = 'D:\Project\opensource\vcpkg\installed\x64-windows-static\lib'
SET OPENSSL_STATIC = 'Yes'
SET OPENSSL_NO_VENDOR=1

 cargo  build --target aarch64-unknown-linux-gnu