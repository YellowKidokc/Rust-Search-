@echo off
setlocal
pushd "%~dp0"
if errorlevel 1 exit /b 1
if not exist "target\release\tpsearch.exe" (
  echo Build tpsearch first: cargo build --release
  popd
  exit /b 1
)
"target\release\tpsearch.exe"  %*
set "tpsearch_exit=%errorlevel%"
popd
exit /b %tpsearch_exit%
