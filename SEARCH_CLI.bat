@echo off
setlocal
pushd "%~dp0"
if errorlevel 1 exit /b 1
python ui\search_cli.py %*
if errorlevel 1 pause
popd
