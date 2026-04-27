# =============================================================================
# tests/db-integration/Makefile — Spindle integration test driver
# =============================================================================
# Targets:
#   make up      — start pg16 + pg14 + mysql8, block until healthy
#   make down    — stop and remove (no volumes are named, so state goes too)
#   make logs    — tail logs
#   make test    — run vcs/specs/L2-standard/database/{postgres,mysql}/
#   make ps      — show running services + health
# =============================================================================

COMPOSE  ?= docker compose
PROJECT  := verum-spindle
COMPOSE_FILE := docker-compose.yml
SERVICES := pg16 pg14 mysql8
PROJECT_FLAGS := -p $(PROJECT) -f $(COMPOSE_FILE)

VTEST_LEVEL  ?= L2
VTEST_FILTER ?= database

.PHONY: up down restart logs test ps health clean

up:
	@echo ">>> bringing up $(SERVICES)"
	$(COMPOSE) $(PROJECT_FLAGS) up -d --wait $(SERVICES)
	@$(MAKE) --no-print-directory health

down:
	$(COMPOSE) $(PROJECT_FLAGS) down -v --remove-orphans

restart: down up

logs:
	$(COMPOSE) $(PROJECT_FLAGS) logs -f --tail=100 $(SERVICES)

ps:
	$(COMPOSE) $(PROJECT_FLAGS) ps

health:
	@echo ">>> readiness probe"
	@for svc in $(SERVICES); do \
	  status=$$($(COMPOSE) $(PROJECT_FLAGS) ps --format '{{.Service}} {{.Health}}' \
	            | awk -v s=$$svc '$$1==s {print $$2}'); \
	  echo "  $$svc: $$status"; \
	done

# Run the adapter VCS tests.  The runner needs the env-var set so the
# test bodies switch from skip→run.
test:
	VERUM_DB_INTEGRATION=1 \
	  PGHOST=127.0.0.1 PGPORT=55432 \
	  PGUSER=spindle  PGPASSWORD=spindle_pw_local_only \
	  PGDATABASE=spindle_test \
	  MYSQL_HOST=127.0.0.1 MYSQL_PORT=53306 \
	  MYSQL_USER=spindle MYSQL_PASSWORD=spindle_pw_local_only \
	  MYSQL_DATABASE=spindle_test \
	  cargo run -p vtest -- run --level $(VTEST_LEVEL) --filter $(VTEST_FILTER)

clean: down
	@echo ">>> nothing else to clean (no named volumes)"
