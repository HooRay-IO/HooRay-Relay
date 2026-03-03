.PHONY: package-ingestion package-worker build-IngestionFunction

package-ingestion:
	mkdir -p ".zig-cache/global" ".zig-cache/local"
	ZIG_GLOBAL_CACHE_DIR="$(PWD)/.zig-cache/global" ZIG_LOCAL_CACHE_DIR="$(PWD)/.zig-cache/local" cargo lambda build --locked --release --arm64 -p ingestion --bin ingestion
	mkdir -p "$(ARTIFACTS_DIR)"
	cp "target/lambda/ingestion/bootstrap" "$(ARTIFACTS_DIR)/bootstrap"
	chmod +x "$(ARTIFACTS_DIR)/bootstrap"

package-worker:
	cargo build --locked --release -p worker --bin worker

build-IngestionFunction: package-ingestion
