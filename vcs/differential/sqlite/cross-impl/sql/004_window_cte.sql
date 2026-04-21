-- Scenario 004: recursive CTE + window function
WITH RECURSIVE fib(n, a, b) AS (
    SELECT 1, 0, 1
    UNION ALL
    SELECT n + 1, b, a + b FROM fib WHERE n < 15
)
SELECT n, a,
       ROW_NUMBER() OVER (ORDER BY a) AS rank,
       LAG(a, 1) OVER (ORDER BY n) AS prev_fib
FROM fib
ORDER BY n;
