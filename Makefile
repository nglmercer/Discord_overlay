TARGET_LINUX := x86_64-unknown-linux-gnu
TARGET_WINDOWS := x86_64-pc-windows-gnu
OUT_DIR := dist

.PHONY: build build-linux build-windows clean

build: build-linux build-windows

build-linux:
	cargo build --release --locked --target $(TARGET_LINUX)
	mkdir -p $(OUT_DIR)
	cp target/$(TARGET_LINUX)/release/discord_overlay $(OUT_DIR)/discord_overlay-linux-x86_64

build-windows:
	rustup target add $(TARGET_WINDOWS) 2>/dev/null || true
	cargo build --release --locked --target $(TARGET_WINDOWS)
	mkdir -p $(OUT_DIR)
	cp target/$(TARGET_WINDOWS)/release/discord_overlay.exe $(OUT_DIR)/discord_overlay-windows-x86_64.exe

clean:
	rm -rf $(OUT_DIR)
