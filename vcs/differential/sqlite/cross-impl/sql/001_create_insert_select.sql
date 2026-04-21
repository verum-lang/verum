-- Scenario 001: baseline CRUD
-- Expected: identical output across C-SQLite and loom.
CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, age INTEGER);
INSERT INTO users VALUES (1, 'Alice', 30);
INSERT INTO users VALUES (2, 'Bob', 25);
INSERT INTO users VALUES (3, 'Carol', 40);
SELECT id, name, age FROM users ORDER BY id;
