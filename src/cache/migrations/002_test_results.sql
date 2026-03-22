CREATE TABLE IF NOT EXISTS test_results (
    id INTEGER PRIMARY KEY,
    test_name TEXT NOT NULL,
    target TEXT NOT NULL,
    cache_key TEXT NOT NULL,
    verdict TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    flake_count INTEGER NOT NULL DEFAULT 0,
    run_count INTEGER NOT NULL DEFAULT 1,
    stdout TEXT,
    stderr TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_test_results_key ON test_results(cache_key);
CREATE INDEX IF NOT EXISTS idx_test_results_name_target ON test_results(test_name, target);
