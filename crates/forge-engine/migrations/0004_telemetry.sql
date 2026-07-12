ALTER TABLE actions ADD COLUMN was_hit INTEGER NOT NULL DEFAULT 0;
ALTER TABLE actions ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS components (
    label   TEXT PRIMARY KEY,
    kind    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS component_edges (
    dependent  TEXT NOT NULL,
    dependency TEXT NOT NULL,
    PRIMARY KEY (dependent, dependency)
);
