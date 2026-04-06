//! Meta Async Executor - Parallel Execution of Meta Functions
//!
//! Implements parallel execution of `meta async fn` using Rayon for CPU-bound parallelism.
//!
//! Meta async functions enable parallel pure computation at compile time.
//! async/await in meta context is for PARALLELISM (multiple CPU cores), NOT for I/O.
//! The meta sandbox FORBIDS all I/O operations even in async meta functions.
//! Use cases: parallel type generation, parallel macro expansion, parallel validation.
//!
//! # CRITICAL: Rayon NOT Tokio
//!
//! Meta async functions use **RAYON** (work-stealing CPU parallelism), NOT **TOKIO** (I/O parallelism).
//! This is because:
//! - Meta functions are compile-time, CPU-bound operations
//! - NO I/O is allowed in meta context
//! - Work-stealing provides better load balancing for independent tasks
//!
//! # Example
//!
//! ```verum
//! // ✅ ALLOWED: Parallel pure computation
//! meta async fn parallel_type_generation() -> List<Type> {
//!     let (branch_a, branch_b) = join!(
//!         async { generate_types_for_module_a() },
//!         async { generate_types_for_module_b() }
//!     ).await;
//!
//!     merge_types(branch_a, branch_b)
//! }
//!
//! // ❌ FORBIDDEN: I/O operations
//! meta async fn fetch_config() -> Config {
//!     http.get("...").await  // COMPILE ERROR: I/O in meta context
//! }
//! ```

use parking_lot::Mutex;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use verum_ast::expr::Expr;
use verum_common::{List, Text};

use super::{ConstValue, MetaContext, MetaError};
use super::registry::MetaFunction;
use super::sandbox::MetaSandbox;

/// Task identifier for dependency tracking
pub type TaskId = usize;

/// A parallel task extracted from a meta async fn
#[derive(Debug, Clone)]
pub struct MetaTask {
    /// Unique task identifier
    pub id: TaskId,
    /// Task name (for debugging)
    pub name: Text,
    /// The expression to evaluate
    pub expr: Expr,
    /// Tasks this task depends on
    pub dependencies: Vec<TaskId>,
}

/// Dependency graph for parallel task execution
#[derive(Debug)]
pub struct TaskDependencyGraph {
    /// Task ID → Tasks it depends on
    dependencies: HashMap<TaskId, HashSet<TaskId>>,
    /// Task ID → Tasks that depend on it
    dependents: HashMap<TaskId, HashSet<TaskId>>,
    /// All task IDs
    all_tasks: HashSet<TaskId>,
}

impl TaskDependencyGraph {
    /// Create a new empty dependency graph
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
            all_tasks: HashSet::new(),
        }
    }

    /// Add a task to the graph
    pub fn add_task(&mut self, task_id: TaskId, deps: Vec<TaskId>) {
        self.all_tasks.insert(task_id);

        for dep in &deps {
            self.all_tasks.insert(*dep);

            self.dependencies.entry(task_id).or_default().insert(*dep);

            self.dependents.entry(*dep).or_default().insert(task_id);
        }

        // Ensure task has an entry even with no deps
        self.dependencies.entry(task_id).or_default();
    }

    /// Validate the graph is a DAG (no cycles)
    pub fn validate_dag(&self) -> Result<(), MetaAsyncError> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for &task_id in &self.all_tasks {
            if self.has_cycle(task_id, &mut visited, &mut rec_stack)? {
                return Err(MetaAsyncError::CyclicDependency);
            }
        }

        Ok(())
    }

    /// Check for cycles starting from a given task
    fn has_cycle(
        &self,
        task_id: TaskId,
        visited: &mut HashSet<TaskId>,
        rec_stack: &mut HashSet<TaskId>,
    ) -> Result<bool, MetaAsyncError> {
        if rec_stack.contains(&task_id) {
            return Ok(true); // Cycle detected
        }

        if visited.contains(&task_id) {
            return Ok(false); // Already processed
        }

        visited.insert(task_id);
        rec_stack.insert(task_id);

        if let Some(deps) = self.dependencies.get(&task_id) {
            for &dep in deps {
                if self.has_cycle(dep, visited, rec_stack)? {
                    return Ok(true);
                }
            }
        }

        rec_stack.remove(&task_id);
        Ok(false)
    }

    /// Topological sort - returns execution order respecting dependencies
    pub fn topological_sort(&self) -> Result<Vec<TaskId>, MetaAsyncError> {
        self.validate_dag()?;

        let mut in_degree: HashMap<TaskId, usize> = HashMap::new();
        for &task_id in &self.all_tasks {
            in_degree.insert(
                task_id,
                self.dependencies.get(&task_id).map_or(0, |d| d.len()),
            );
        }

        // Start with tasks that have no dependencies
        let mut queue: Vec<TaskId> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(id, _)| *id)
            .collect();

        let mut result = Vec::new();

        while let Some(task_id) = queue.pop() {
            result.push(task_id);

            if let Some(dependents) = self.dependents.get(&task_id) {
                for &dependent in dependents {
                    if let Some(deg) = in_degree.get_mut(&dependent) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(dependent);
                        }
                    }
                }
            }
        }

        if result.len() != self.all_tasks.len() {
            return Err(MetaAsyncError::CyclicDependency);
        }

        Ok(result)
    }

    /// Get tasks that can be executed in parallel (no unmet dependencies)
    pub fn get_ready_tasks(&self, completed: &HashSet<TaskId>) -> Vec<TaskId> {
        self.all_tasks
            .iter()
            .filter(|&task_id| {
                // Not yet completed
                !completed.contains(task_id) &&
                // All dependencies completed
                self.dependencies
                    .get(task_id)
                    .map_or(true, |deps| deps.iter().all(|d| completed.contains(d)))
            })
            .copied()
            .collect()
    }
}

impl Default for TaskDependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors from meta async execution
#[derive(Debug, Clone)]
pub enum MetaAsyncError {
    /// I/O operation detected in meta async context
    IoDetected { task_name: Text, operation: Text },
    /// Cyclic dependency detected
    CyclicDependency,
    /// Task execution failed
    TaskFailed { task_id: TaskId, error: Text },
    /// Timeout exceeded
    Timeout,
    /// Stack overflow
    StackOverflow,
    /// Other error
    Other(Text),
}

impl std::fmt::Display for MetaAsyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetaAsyncError::IoDetected {
                task_name,
                operation,
            } => {
                write!(
                    f,
                    "I/O operation '{}' detected in meta async task '{}'",
                    operation, task_name
                )
            }
            MetaAsyncError::CyclicDependency => {
                write!(f, "Cyclic dependency detected in meta async tasks")
            }
            MetaAsyncError::TaskFailed { task_id, error } => {
                write!(f, "Meta async task {} failed: {}", task_id, error)
            }
            MetaAsyncError::Timeout => write!(f, "Meta async execution timeout"),
            MetaAsyncError::StackOverflow => write!(f, "Meta async stack overflow"),
            MetaAsyncError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for MetaAsyncError {}

/// Executor for meta async functions using Rayon
///
/// # Thread Safety
/// Uses work-stealing parallelism via Rayon thread pool
#[allow(dead_code)] // Fields are infrastructure for future parallelism features
pub struct MetaAsyncExecutor {
    /// Sandbox for I/O validation
    sandbox: MetaSandbox,
    /// Task dependency graph
    task_graph: TaskDependencyGraph,
    /// Maximum parallel tasks
    max_parallelism: usize,
    /// Execution timeout in milliseconds
    timeout_ms: u64,
}

impl MetaAsyncExecutor {
    /// Create a new executor with default settings
    pub fn new() -> Self {
        Self {
            sandbox: MetaSandbox::new(),
            task_graph: TaskDependencyGraph::new(),
            max_parallelism: rayon::current_num_threads(),
            timeout_ms: 30_000, // 30 seconds default
        }
    }

    /// Create with custom parallelism settings
    pub fn with_parallelism(max_parallelism: usize, timeout_ms: u64) -> Self {
        Self {
            sandbox: MetaSandbox::new(),
            task_graph: TaskDependencyGraph::new(),
            max_parallelism,
            timeout_ms,
        }
    }

    /// Execute a meta async function
    ///
    /// 1. Validate no I/O operations
    /// 2. Extract parallel tasks
    /// 3. Build dependency graph
    /// 4. Execute on Rayon thread pool
    pub fn execute_meta_async_fn(
        &mut self,
        func: &MetaFunction,
        args: List<ConstValue>,
        context: &mut MetaContext,
    ) -> Result<ConstValue, MetaAsyncError> {
        tracing::debug!("Executing meta async fn: {}", func.name.as_str());

        // Validate no I/O in the function
        self.validate_no_io(&func.body)?;

        // For simple cases without explicit parallelism, execute sequentially
        // In production, this would extract join!/join_all! patterns
        self.execute_sequentially(func, args, context)
    }

    /// Execute multiple tasks in parallel respecting dependencies
    pub fn execute_parallel_tasks(
        &self,
        tasks: &[MetaTask],
        context: Arc<Mutex<MetaContext>>,
    ) -> Result<HashMap<TaskId, ConstValue>, MetaAsyncError> {
        // Validate no I/O in all tasks
        for task in tasks {
            self.validate_no_io(&task.expr)?;
        }

        // Build dependency graph
        let mut graph = TaskDependencyGraph::new();
        for task in tasks {
            graph.add_task(task.id, task.dependencies.clone());
        }
        graph.validate_dag()?;

        // Execute using Rayon
        let results: Arc<Mutex<HashMap<TaskId, ConstValue>>> = Arc::new(Mutex::new(HashMap::new()));
        let completed: Arc<Mutex<HashSet<TaskId>>> = Arc::new(Mutex::new(HashSet::new()));
        let errors: Arc<Mutex<Option<MetaAsyncError>>> = Arc::new(Mutex::new(None));

        // Process in waves based on dependencies
        loop {
            // Get ready tasks
            let ready = {
                let completed_guard = completed.lock();
                graph.get_ready_tasks(&completed_guard)
            };

            if ready.is_empty() {
                // Check if all done
                let completed_guard = completed.lock();
                if completed_guard.len() == tasks.len() {
                    break;
                }
                // Check for errors
                if errors.lock().is_some() {
                    break;
                }
                // Shouldn't happen if graph is valid
                break;
            }

            // Execute ready tasks in parallel
            ready.par_iter().for_each(|&task_id| {
                // Skip if error already occurred
                if errors.lock().is_some() {
                    return;
                }

                // Find the task
                if let Some(task) = tasks.iter().find(|t| t.id == task_id) {
                    // Execute task
                    let result = {
                        let mut ctx = context.lock();
                        // In production, this would use the sandbox to evaluate
                        self.execute_task_expr(&task.expr, &mut ctx)
                    };

                    match result {
                        Ok(value) => {
                            results.lock().insert(task_id, value);
                            completed.lock().insert(task_id);
                        }
                        Err(e) => {
                            *errors.lock() = Some(MetaAsyncError::TaskFailed {
                                task_id,
                                error: Text::from(format!("{:?}", e)),
                            });
                        }
                    }
                }
            });
        }

        // Check for errors
        if let Some(error) = errors.lock().take() {
            return Err(error);
        }

        // Extract results
        let final_results = results.lock().clone();
        Ok(final_results)
    }

    /// Validate that an expression contains no I/O operations
    fn validate_no_io(&self, expr: &Expr) -> Result<(), MetaAsyncError> {
        // For now, we just validate the AST structure
        // In a full implementation, this would use the sandbox to check for I/O
        // The sandbox.execute_expr is private and requires a MetaContext
        // So we'll do a simpler AST-based check
        self.check_expr_for_io(expr)
    }

    /// Check expression AST for I/O operations
    fn check_expr_for_io(&self, expr: &Expr) -> Result<(), MetaAsyncError> {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                // Check if function call is to an I/O function
                if let ExprKind::Path(path) = &func.kind {
                    if let Some(name) = path.as_ident() {
                        let name_str = name.as_str();
                        if name_str.contains("read")
                            || name_str.contains("write")
                            || name_str.contains("File")
                            || name_str.contains("http")
                            || name_str.contains("net")
                            || name_str.contains("spawn")
                        {
                            return Err(MetaAsyncError::IoDetected {
                                task_name: Text::from("<async task>"),
                                operation: Text::from(name_str),
                            });
                        }
                    }
                }
                // Recurse into arguments
                for arg in args.iter() {
                    self.check_expr_for_io(arg)?;
                }
                Ok(())
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr_for_io(receiver)?;
                for arg in args.iter() {
                    self.check_expr_for_io(arg)?;
                }
                Ok(())
            }
            ExprKind::Binary { left, right, .. } => {
                self.check_expr_for_io(left)?;
                self.check_expr_for_io(right)?;
                Ok(())
            }
            ExprKind::Unary { expr: inner, .. } => self.check_expr_for_io(inner),
            ExprKind::Block(block) => {
                for stmt in block.stmts.iter() {
                    // Check statements for I/O
                    match &stmt.kind {
                        verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                            self.check_expr_for_io(&e)?;
                        }
                        verum_ast::stmt::StmtKind::Let { value, .. } => {
                            if let Some(v) = value {
                                self.check_expr_for_io(v)?;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(e) = &block.expr {
                    self.check_expr_for_io(&e)?;
                }
                Ok(())
            }
            _ => Ok(()), // Other expressions are safe
        }
    }

    /// Execute a task expression
    fn execute_task_expr(
        &self,
        expr: &Expr,
        context: &mut MetaContext,
    ) -> Result<ConstValue, MetaError> {
        // Convert to meta expr and evaluate
        let meta_expr = context.ast_expr_to_meta_expr(expr)?;
        context.eval_meta_expr(&meta_expr)
    }

    /// Execute sequentially (fallback for simple cases)
    fn execute_sequentially(
        &self,
        func: &MetaFunction,
        args: List<ConstValue>,
        context: &mut MetaContext,
    ) -> Result<ConstValue, MetaAsyncError> {
        // Convert List to Vec for compatibility
        let args_vec: Vec<ConstValue> = args.iter().cloned().collect();
        context
            .execute_user_meta_fn(func, args_vec)
            .map_err(|e| MetaAsyncError::Other(Text::from(e.to_string())))
    }
}

impl Default for MetaAsyncExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for parallel task execution
#[derive(Debug)]
pub struct ParallelTaskBuilder {
    tasks: Vec<MetaTask>,
    next_id: TaskId,
}

impl ParallelTaskBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            next_id: 0,
        }
    }

    /// Add a task with no dependencies
    pub fn add_task(&mut self, name: &str, expr: Expr) -> TaskId {
        self.add_task_with_deps(name, expr, vec![])
    }

    /// Add a task with dependencies
    pub fn add_task_with_deps(&mut self, name: &str, expr: Expr, deps: Vec<TaskId>) -> TaskId {
        let id = self.next_id;
        self.next_id += 1;

        self.tasks.push(MetaTask {
            id,
            name: Text::from(name),
            expr,
            dependencies: deps,
        });

        id
    }

    /// Build and get tasks
    pub fn build(self) -> Vec<MetaTask> {
        self.tasks
    }
}

impl Default for ParallelTaskBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::Span;
    use verum_ast::{Literal, LiteralKind};

    #[test]
    fn test_dependency_graph_creation() {
        let mut graph = TaskDependencyGraph::new();

        // Task 0: no deps
        // Task 1: depends on 0
        // Task 2: depends on 0
        // Task 3: depends on 1, 2
        graph.add_task(0, vec![]);
        graph.add_task(1, vec![0]);
        graph.add_task(2, vec![0]);
        graph.add_task(3, vec![1, 2]);

        assert!(graph.validate_dag().is_ok());
    }

    #[test]
    fn test_dependency_graph_cycle_detection() {
        let mut graph = TaskDependencyGraph::new();

        // Create cycle: 0 -> 1 -> 2 -> 0
        graph.add_task(0, vec![2]);
        graph.add_task(1, vec![0]);
        graph.add_task(2, vec![1]);

        assert!(matches!(
            graph.validate_dag(),
            Err(MetaAsyncError::CyclicDependency)
        ));
    }

    #[test]
    fn test_topological_sort() {
        let mut graph = TaskDependencyGraph::new();

        graph.add_task(0, vec![]);
        graph.add_task(1, vec![0]);
        graph.add_task(2, vec![0]);
        graph.add_task(3, vec![1, 2]);

        let order = graph.topological_sort().unwrap();

        // 0 must come first
        assert_eq!(order[0], 0);
        // 3 must come last
        assert_eq!(order[3], 3);
        // 1 and 2 must come after 0 and before 3
        let pos_1 = order.iter().position(|&x| x == 1).unwrap();
        let pos_2 = order.iter().position(|&x| x == 2).unwrap();
        assert!(pos_1 > 0 && pos_1 < 3);
        assert!(pos_2 > 0 && pos_2 < 3);
    }

    #[test]
    fn test_get_ready_tasks() {
        let mut graph = TaskDependencyGraph::new();

        graph.add_task(0, vec![]);
        graph.add_task(1, vec![0]);
        graph.add_task(2, vec![0]);
        graph.add_task(3, vec![1, 2]);

        // Initially only task 0 is ready
        let ready = graph.get_ready_tasks(&HashSet::new());
        assert_eq!(ready, vec![0]);

        // After 0 completes, 1 and 2 are ready
        let mut completed = HashSet::new();
        completed.insert(0);
        let mut ready = graph.get_ready_tasks(&completed);
        ready.sort();
        assert_eq!(ready, vec![1, 2]);

        // After 1 and 2 complete, 3 is ready
        completed.insert(1);
        completed.insert(2);
        let ready = graph.get_ready_tasks(&completed);
        assert_eq!(ready, vec![3]);
    }

    fn dummy_expr() -> Expr {
        let span = Span::default();
        let lit = Literal {
            kind: LiteralKind::Bool(false),
            span,
        };
        Expr::literal(lit)
    }

    #[test]
    fn test_parallel_task_builder() {
        let mut builder = ParallelTaskBuilder::new();

        let t0 = builder.add_task("init", dummy_expr());
        let t1 = builder.add_task_with_deps("process_a", dummy_expr(), vec![t0]);
        let t2 = builder.add_task_with_deps("process_b", dummy_expr(), vec![t0]);
        let _t3 = builder.add_task_with_deps("merge", dummy_expr(), vec![t1, t2]);

        let tasks = builder.build();
        assert_eq!(tasks.len(), 4);
        assert_eq!(tasks[0].name.as_str(), "init");
        assert!(tasks[0].dependencies.is_empty());
        assert_eq!(tasks[3].dependencies, vec![1, 2]);
    }

    #[test]
    fn test_executor_creation() {
        let executor = MetaAsyncExecutor::new();
        assert!(executor.max_parallelism > 0);
        assert_eq!(executor.timeout_ms, 30_000);
    }

    #[test]
    fn test_executor_with_custom_settings() {
        let executor = MetaAsyncExecutor::with_parallelism(4, 10_000);
        assert_eq!(executor.max_parallelism, 4);
        assert_eq!(executor.timeout_ms, 10_000);
    }
}
