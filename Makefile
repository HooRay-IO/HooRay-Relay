.PHONY: package-ingestion package-worker build-IngestionFunction

package-ingestion:
	cargo lambda build --locked --release --arm64 -p ingestion --bin ingestion
	mkdir -p "$(ARTIFACTS_DIR)"
	cp "target/lambda/ingestion/bootstrap" "$(ARTIFACTS_DIR)/bootstrap"
	chmod +x "$(ARTIFACTS_DIR)/bootstrap"

package-worker:
	cargo build --locked --release -p worker --bin worker

build-IngestionFunction: package-ingestion
