.PHONY: web-build build run

web-build:
	cd web && pnpm install --frozen-lockfile
	cd web && pnpm build

build: web-build
	cargo build --release --locked --bin gsdv

run: web-build
	cargo run --release --bin gsdv
