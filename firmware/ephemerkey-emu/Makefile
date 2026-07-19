# ephemerkey emulator (host)

.PHONY: build test run demo

build:
	cargo build

# core-crate unit tests (RFC vectors, delay windows, cadence, decoy, replay)
test:
	cd ../ephemerkey-core && cargo test --features introspect

# interactive REPL against the demo scenario
run:
	cargo run -q -- scenarios/walk-vault.json

# self-checking end-to-end rituals (exit non-zero on any failed expect)
demo:
	cargo run -q -- scenarios/walk-vault.json < scripts/walk-vault.txt
	cargo run -q -- scenarios/courier.json < scripts/courier.txt
	cargo run -q -- scenarios/deadman.json < scripts/deadman.txt
	cargo run -q -- scenarios/quorum.json < scripts/quorum.txt
	cargo run -q -- scenarios/portable-vault.json < scripts/portable-vault.txt
