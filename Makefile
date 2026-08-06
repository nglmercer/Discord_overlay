TARGET_LINUX := x86_64-unknown-linux-gnu
TARGET_WINDOWS := x86_64-pc-windows-gnu
TARGET_WINDOWS_MSVC := x86_64-pc-windows-msvc
OUT_DIR := dist

.PHONY: build build-linux build-windows build-windows-msvc prepare-dist clean

build: build-linux build-windows build-windows-msvc

build-linux: prepare-dist
	cargo build --release --locked --target $(TARGET_LINUX)
	cp target/$(TARGET_LINUX)/release/discord_overlay $(OUT_DIR)/discord_overlay-linux-x86_64

build-windows: prepare-dist
	rustup target add $(TARGET_WINDOWS) 2>/dev/null || true
	cargo build --release --locked --target $(TARGET_WINDOWS)
	cp target/$(TARGET_WINDOWS)/release/discord_overlay.exe $(OUT_DIR)/discord_overlay-windows-x86_64.exe

# Cross-compiles the MSVC ABI target from Linux using clang/lld-link plus the
# Windows SDK/CRT that cargo-xwin downloads and caches.
build-windows-msvc: prepare-dist
	command -v cargo-xwin >/dev/null || cargo install cargo-xwin
	rustup target add $(TARGET_WINDOWS_MSVC) 2>/dev/null || true
	cargo xwin build --release --locked --target $(TARGET_WINDOWS_MSVC)
	cp target/$(TARGET_WINDOWS_MSVC)/release/discord_overlay.exe $(OUT_DIR)/discord_overlay-windows-x86_64-msvc.exe

# Release binaries are self-contained at runtime. Keep the editable config,
# usage guide, and image folder beside each executable in dist/.
prepare-dist:
	mkdir -p $(OUT_DIR)/assets
	cp config.toml README.md $(OUT_DIR)/
	cp -a assets/. $(OUT_DIR)/assets/

clean:
	rm -rf $(OUT_DIR)
