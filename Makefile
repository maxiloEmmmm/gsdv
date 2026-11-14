.PHONY: web-build build run run-tui pprof-run

fps ?= 4

web-build:
	cd web && pnpm install --frozen-lockfile
	cd web && pnpm build

build: web-build
	cargo build --release --locked --bin gsdv

run: web-build
	cargo run --release --bin gsdv

run-tui:
	cargo run --release --bin gsdv -- tui --fps $(fps)

pprof-run: web-build
	RUSTFLAGS="-C force-frame-pointers=yes" cargo run --release --features pprof-run --bin gsdv
