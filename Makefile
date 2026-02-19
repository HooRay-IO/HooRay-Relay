.PHONY: build-rust-binaries package-ingestion package-worker build-IngestionFunction build-WorkerFunction

build-rust-binaries:
	cargo build --release -p ingestion -p worker
	rm -rf "$(ARTIFACTS_DIR)"

package-ingestion: build-rust-binaries
	mkdir -p "$(ARTIFACTS_DIR)"
	cp "target/release/ingestion" "$(ARTIFACTS_DIR)/bootstrap"
	chmod +x "$(ARTIFACTS_DIR)/bootstrap"

package-worker: build-rust-binaries
	mkdir -p "$(ARTIFACTS_DIR)"
	cp "target/release/worker" "$(ARTIFACTS_DIR)/bootstrap"
	chmod +x "$(ARTIFACTS_DIR)/bootstrap"

build-IngestionFunction: package-ingestion

build-WorkerFunction: package-worker
