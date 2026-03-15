CREATE TABLE IF NOT EXISTS artifacts (
    id INTEGER PRIMARY KEY,
    hash TEXT UNIQUE NOT NULL,
    size INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    last_accessed INTEGER NOT NULL,
    compressed INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS rules (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    hash TEXT NOT NULL,
    artifact_id INTEGER,
    FOREIGN KEY(artifact_id) REFERENCES artifacts(id)
);

CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    mtime INTEGER,
    artifact_id INTEGER,
    FOREIGN KEY(artifact_id) REFERENCES artifacts(id)
);

CREATE INDEX IF NOT EXISTS idx_artifacts_hash ON artifacts(hash);
CREATE INDEX IF NOT EXISTS idx_rules_name ON rules(name);
CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
