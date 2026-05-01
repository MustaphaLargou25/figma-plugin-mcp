@echo off
REM Starts the Rust Claude<->Figma WebSocket relay on localhost:3055.
REM Build artifacts are placed under rust-relay\target.
title Claude Figma Rust Relay (port 3055)
pushd "%~dp0rust-relay"
cargo run --release
popd
pause
