@echo off
REM Starts the Claude<->Figma WebSocket relay on localhost:3055.
REM Leave this window open while you work with Figma write tools.
REM Press Ctrl+C to stop.
title Claude Figma Socket Relay (port 3055)
"C:\Users\hp\.bun\bin\bun.exe" "%~dp0server\socket.js"
pause
