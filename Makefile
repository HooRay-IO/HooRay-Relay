.PHONY: package-ingestion package-worker build-IngestionFunction build-WorkerFunction

package-ingestion:
	cargo build --locked --release -p ingestion --bin ingestion
	mkdir -p "$(ARTIFACTS_DIR)"
	cp "target/release/ingestion" "$(ARTIFACTS_DIR)/bootstrap"
	chmod +x "$(ARTIFACTS_DIR)/bootstrap"

package-worker:
	cargo build --locked --release -p worker --bin worker
	mkdir -p "$(ARTIFACTS_DIR)"
	cp "target/release/worker" "$(ARTIFACTS_DIR)/bootstrap"
	chmod +x "$(ARTIFACTS_DIR)/bootstrap"

build-IngestionFunction: package-ingestion

build-WorkerFunction: package-worker
