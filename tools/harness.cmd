@echo off
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
if errorlevel 1 exit /b 1
set "PATH=C:\Users\teusm\Documents\rea\scarlet_nexus_rust\tools;C:\Users\teusm\Documents\rea\scarlet_nexus_rust\rust\save-data-names\target\release;%PATH%"
rust-anything %*
