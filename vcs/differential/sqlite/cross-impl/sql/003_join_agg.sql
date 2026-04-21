-- Scenario 003: JOIN + GROUP BY + HAVING
CREATE TABLE orders (id INT, customer_id INT, total REAL);
CREATE TABLE customers (id INT, name TEXT);

INSERT INTO customers VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol');
INSERT INTO orders VALUES
    (1, 1, 100.00),
    (2, 1, 200.50),
    (3, 2, 75.00),
    (4, 2, 120.00),
    (5, 3, 1000.00);

SELECT c.name, COUNT(o.id) AS n_orders, SUM(o.total) AS total_spent
FROM customers c
INNER JOIN orders o ON o.customer_id = c.id
GROUP BY c.id, c.name
HAVING SUM(o.total) > 100
ORDER BY total_spent DESC;
