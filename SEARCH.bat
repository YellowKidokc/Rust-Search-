@echo off
setlocal
pushd "%~dp0"
if errorlevel 1 exit /b 1
title tpsearch - Research Search
python ui\gui.py %*
if errorlevel 1 pause
popd
