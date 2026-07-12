ALTER TABLE test_results ADD COLUMN flake_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE test_results ADD COLUMN run_count INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS test_history (
    component   TEXT NOT NULL,
    cache_key   TEXT NOT NULL,
    verdict     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_test_history_component ON test_history(component);

CREATE TABLE IF NOT EXISTS action_inputs (
    cache_key TEXT NOT NULL,
    path      TEXT NOT NULL,
    hash      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_action_inputs_key ON action_inputs(cache_key);
