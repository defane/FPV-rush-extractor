# FPV Rush — native Rust GUI. Installers are built on their destination OS.
.DEFAULT_GOAL := help
.PHONY: help tools deps check test app run cli media-tools package linux macos windows macos-remote clean dist-clean

ifeq ($(wildcard fpv-rs/Cargo.toml),)
$(error run make from the repository root)
endif

HOST := $(shell uname -s)
TARGET ?=
GIT_REF ?= $(shell git branch --show-current)
MAC_ARCH := $(if $(TARGET),$(patsubst %-apple-darwin,%,$(TARGET)),universal)
TARGET_FLAG := $(if $(TARGET),--target $(TARGET))
export PATH := $(HOME)/.cargo/bin:$(PATH)

help: ## list build targets
	@echo "FPV Rush — native Rust GUI (no Node or WebView)"
	@awk -F ':.*## ' '/^[a-z-]+:.*## / {printf "  %-14s %s\n", $$1, $$2}' Makefile

tools: ## check Rust and the native build tools
	@command -v cargo >/dev/null || { echo "Install stable Rust with rustup"; exit 1; }
	@command -v cc >/dev/null || { echo "Install a C compiler (SQLite and FFmpeg build tools)"; exit 1; }

deps: tools ## no frontend package installation; Cargo resolves dependencies
	@echo "Rust dependencies are recorded in fpv-rs/Cargo.lock."

check: ## clippy and all Rust tests (test fixtures require ffmpeg with libx264 on PATH)
	cd fpv-rs && cargo clippy --workspace --all-targets --locked -- -D warnings
	cd fpv-rs && cargo test --workspace --locked

test: ## run every Rust test
	cd fpv-rs && cargo test --workspace --locked

app: tools ## compile the native desktop app
	cd fpv-rs && cargo build --release --locked -p fpv-app $(TARGET_FLAG)

run: ## start the native desktop app with prepared bundled media tools
	cd fpv-rs && PATH="../packaging/resources/bin:$$PATH" cargo run --locked -p fpv-app

cli: tools ## compile the headless CLI
	cd fpv-rs && cargo build --release --locked -p fpvlib $(TARGET_FLAG)

media-tools: ## build LGPL-only FFmpeg/ffprobe and bundle their corresponding sources
	bash packaging/build-ffmpeg.sh $(if $(filter Darwin,$(HOST)),$(MAC_ARCH),native)

package: ## build installers for this host OS
ifeq ($(HOST),Darwin)
	$(MAKE) macos
else ifeq ($(HOST),Linux)
	$(MAKE) linux
else
	$(MAKE) windows
endif

linux: ## Linux deb, tar.gz and AppImage (downloads verified packaging tools)
	@test "$(HOST)" = Linux || { echo "Build Linux packages on Linux"; exit 1; }
	$(MAKE) media-tools
	bash packaging/package.sh linux native

macos: ## macOS DMG (universal Intel + Apple Silicon by default)
	@test "$(HOST)" = Darwin || { echo "Use make macos-remote from this host"; exit 1; }
	$(MAKE) media-tools
	bash packaging/package.sh macos $(MAC_ARCH)

windows: ## Windows installer and portable ZIP (run on Windows with NSIS/MSYS2)
	powershell -ExecutionPolicy Bypass -File packaging/windows/package.ps1

macos-remote: ## dispatch the native macOS build on GitHub Actions
	gh workflow run packaging.yml --ref "$(GIT_REF)" -f platform=macos
	@echo "Follow with: gh run watch"

dist-clean: ## remove generated installers
	$(RM) -r dist

clean: dist-clean ## remove generated Rust build products
	cd fpv-rs && cargo clean
