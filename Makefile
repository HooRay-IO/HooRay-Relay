.PHONY: package-rust-lambdas build-IngestionFunction build-WorkerFunction

package-rust-lambdas:
	cargo build --release -p ingestion -p worker
	mkdir -p "$(ARTIFACTS_DIR)"
	cp "target/release/ingestion" "$(ARTIFACTS_DIR)/ingestion"
	cp "target/release/worker" "$(ARTIFACTS_DIR)/worker"
	printf '%s\n' '#!/bin/sh' \
		'case "$$AWS_LAMBDA_FUNCTION_NAME" in' \
		'  *worker*) exec /var/task/worker ;;' \
		'  *) exec /var/task/ingestion ;;' \
		'esac' > "$(ARTIFACTS_DIR)/bootstrap"
	chmod +x "$(ARTIFACTS_DIR)/bootstrap"
	chmod +x "$(ARTIFACTS_DIR)/ingestion"
	chmod +x "$(ARTIFACTS_DIR)/worker"

build-IngestionFunction: package-rust-lambdas

build-WorkerFunction: package-rust-lambdas
