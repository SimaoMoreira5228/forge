CREATE TABLE IF NOT EXISTS actions (
    cache_key     TEXT PRIMARY KEY,
    component     TEXT NOT NULL,
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_accessed INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_actions_last_accessed ON actions(last_accessed);

CREATE TABLE IF NOT EXISTS test_results (
    cache_key   TEXT PRIMARY KEY,
    component   TEXT NOT NULL,
    verdict     TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_test_results_component ON test_results(component);
