-- Scenario 002: transaction commit and rollback
CREATE TABLE t (x INT);
BEGIN;
INSERT INTO t VALUES (1), (2), (3);
COMMIT;
BEGIN;
INSERT INTO t VALUES (4), (5);
ROLLBACK;
SELECT COUNT(*) FROM t;
SELECT x FROM t ORDER BY x;
