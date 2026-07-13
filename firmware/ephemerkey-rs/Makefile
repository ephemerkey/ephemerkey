# ephemerkey-rs (STM32U083, Embassy)

.PHONY: build release run flash size clippy fmt clean

build:
	cargo build

release:
	cargo build --release

# flash debug build and attach defmt console
run:
	cargo run

flash: release
	probe-rs download --chip STM32U083KC target/thumbv6m-none-eabi/release/ephemerkey-rs
	probe-rs reset --chip STM32U083KC

size: release
	cargo size --release 2>/dev/null || \
		size target/thumbv6m-none-eabi/release/ephemerkey-rs

clippy:
	cargo clippy -- -D warnings

fmt:
	cargo fmt

clean:
	cargo clean
