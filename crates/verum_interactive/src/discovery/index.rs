//! Discovery index for core/ modules and types

use std::collections::HashMap;

use super::docs::{DocEntry, DocKind};
use super::examples::Example;

/// Information about a module
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Module name (e.g., "collections")
    pub name: String,
    /// Full path (e.g., "core.collections")
    pub path: String,
    /// Brief description
    pub description: String,
    /// Layer number (0-8 for math layers)
    pub layer: Option<u8>,
    /// Child modules
    pub children: Vec<String>,
    /// Types defined in this module
    pub types: Vec<String>,
    /// Functions defined in this module
    pub functions: Vec<String>,
}

impl ModuleInfo {
    /// Create a new module info
    pub fn new(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            description: String::new(),
            layer: None,
            children: Vec::new(),
            types: Vec::new(),
            functions: Vec::new(),
        }
    }

    /// Set description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set layer
    pub fn with_layer(mut self, layer: u8) -> Self {
        self.layer = Some(layer);
        self
    }

    /// Add child modules
    pub fn with_children(mut self, children: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.children.extend(children.into_iter().map(|c| c.into()));
        self
    }

    /// Add types
    pub fn with_types(mut self, types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.types.extend(types.into_iter().map(|t| t.into()));
        self
    }

    /// Add functions
    pub fn with_functions(mut self, funcs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.functions.extend(funcs.into_iter().map(|f| f.into()));
        self
    }
}

/// Hierarchical module tree for navigation
#[derive(Debug, Clone)]
pub struct ModuleTree {
    /// All modules by path
    pub modules: HashMap<String, ModuleInfo>,
    /// Root module paths
    pub roots: Vec<String>,
}

impl Default for ModuleTree {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleTree {
    /// Create an empty module tree
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            roots: Vec::new(),
        }
    }

    /// Add a module to the tree
    pub fn add_module(&mut self, module: ModuleInfo) {
        let path = module.path.clone();

        // Check if this is a root module
        if (!path.contains('.') || path.starts_with("core.") && path.matches('.').count() == 1)
            && !self.roots.contains(&path) {
                self.roots.push(path.clone());
            }

        self.modules.insert(path, module);
    }

    /// Get a module by path
    pub fn get(&self, path: &str) -> Option<&ModuleInfo> {
        self.modules.get(path)
    }

    /// List children of a module
    pub fn children(&self, path: &str) -> Vec<&ModuleInfo> {
        self.modules
            .get(path)
            .map(|m| {
                m.children
                    .iter()
                    .filter_map(|c| self.modules.get(c))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all modules at a given depth
    pub fn at_depth(&self, depth: usize) -> Vec<&ModuleInfo> {
        self.modules
            .values()
            .filter(|m| m.path.matches('.').count() == depth)
            .collect()
    }
}

/// Main discovery index containing all searchable documentation
#[derive(Debug)]
pub struct DiscoveryIndex {
    /// Documentation entries by name
    pub docs: HashMap<String, DocEntry>,
    /// Module tree
    pub modules: ModuleTree,
    /// Examples by title
    pub examples: HashMap<String, Example>,
    /// Index of tags to items
    pub tag_index: HashMap<String, Vec<String>>,
    /// Index of types to their methods
    pub method_index: HashMap<String, Vec<String>>,
}

impl Default for DiscoveryIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscoveryIndex {
    /// Create an empty discovery index
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            modules: ModuleTree::new(),
            examples: HashMap::new(),
            tag_index: HashMap::new(),
            method_index: HashMap::new(),
        }
    }

    /// Create the standard library index
    pub fn standard_library() -> Self {
        let mut index = Self::new();
        index.populate_core_modules();
        index.populate_core_types();
        index.populate_examples();
        index
    }

    /// Add a documentation entry
    pub fn add_doc(&mut self, doc: DocEntry) {
        // Index tags
        for tag in &doc.tags {
            self.tag_index
                .entry(tag.clone())
                .or_default()
                .push(doc.name.clone());
        }

        // Index methods
        if doc.kind == DocKind::Method {
            let type_name = doc.module.clone();
            self.method_index
                .entry(type_name)
                .or_default()
                .push(doc.name.clone());
        }

        self.docs.insert(doc.name.clone(), doc);
    }

    /// Add a module
    pub fn add_module(&mut self, module: ModuleInfo) {
        self.modules.add_module(module);
    }

    /// Add an example
    pub fn add_example(&mut self, example: Example) {
        // Index tags
        for tag in &example.tags {
            self.tag_index
                .entry(tag.clone())
                .or_default()
                .push(example.title.clone());
        }

        self.examples.insert(example.title.clone(), example);
    }

    /// Get documentation for a name
    pub fn get_doc(&self, name: &str) -> Option<&DocEntry> {
        self.docs.get(name)
    }

    /// Get all items with a tag
    pub fn by_tag(&self, tag: &str) -> Vec<&str> {
        self.tag_index
            .get(tag)
            .map(|items| items.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get methods for a type
    pub fn methods_for(&self, type_name: &str) -> Vec<&DocEntry> {
        self.method_index
            .get(type_name)
            .map(|methods| {
                methods
                    .iter()
                    .filter_map(|m| self.docs.get(m))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all types
    pub fn all_types(&self) -> Vec<&DocEntry> {
        self.docs
            .values()
            .filter(|d| d.kind == DocKind::Type)
            .collect()
    }

    /// List all protocols
    pub fn all_protocols(&self) -> Vec<&DocEntry> {
        self.docs
            .values()
            .filter(|d| d.kind == DocKind::Protocol)
            .collect()
    }

    /// List all functions
    pub fn all_functions(&self) -> Vec<&DocEntry> {
        self.docs
            .values()
            .filter(|d| d.kind == DocKind::Function)
            .collect()
    }

    /// Populate the core module hierarchy
    fn populate_core_modules(&mut self) {
        // Layer 0: Base
        self.add_module(
            ModuleInfo::new("base", "core.base")
                .with_description("Foundation types and protocols")
                .with_layer(0)
                .with_types(["Maybe", "Result", "Ordering", "Heap", "Shared", "Weak"])
                .with_functions(["panic", "assert", "unreachable"]),
        );

        // Layer 1: Text
        self.add_module(
            ModuleInfo::new("text", "core.text")
                .with_description("Text processing and formatting")
                .with_layer(1)
                .with_types(["Text", "Char", "Regex"])
                .with_functions(["format", "parse"]),
        );

        // Layer 2: Collections
        self.add_module(
            ModuleInfo::new("collections", "core.collections")
                .with_description("Collection types and algorithms")
                .with_layer(2)
                .with_types(["List", "Map", "Set", "Deque", "BTreeMap", "BinaryHeap"])
                .with_functions(["sort", "binary_search"]),
        );

        // Layer 3: I/O
        self.add_module(
            ModuleInfo::new("io", "core.io")
                .with_description("Input/output operations")
                .with_layer(3)
                .with_types(["File", "Path", "BufReader", "BufWriter"])
                .with_functions(["read_file", "write_file", "stdin", "stdout"]),
        );

        // Layer 4: Async
        self.add_module(
            ModuleInfo::new("async", "core.async")
                .with_description("Asynchronous programming")
                .with_layer(4)
                .with_types(["Future", "Task", "Channel", "Stream", "Nursery"])
                .with_functions(["spawn", "select", "join", "timeout"]),
        );

        // Layer 5: Network
        self.add_module(
            ModuleInfo::new("net", "core.net")
                .with_description("Network operations")
                .with_layer(5)
                .with_types(["TcpStream", "UdpSocket", "TcpListener"])
                .with_functions(["connect", "listen", "dns_lookup"]),
        );

        // Math layers
        self.add_module(
            ModuleInfo::new("math", "core.math")
                .with_description("9-layer math stack from IEEE 754 to AI agents")
                .with_children([
                    "core.math.numbers",
                    "core.math.elementary",
                    "core.math.linalg",
                    "core.math.calculus",
                    "core.math.tensor",
                    "core.math.gpu",
                    "core.math.autodiff",
                    "core.math.neural",
                    "core.math.agents",
                ]),
        );

        self.add_module(
            ModuleInfo::new("numbers", "core.math.numbers")
                .with_description("Layer 0: IEEE 754 numerics")
                .with_layer(0)
                .with_types(["Float32", "Float64", "Complex"]),
        );

        self.add_module(
            ModuleInfo::new("elementary", "core.math.elementary")
                .with_description("Layer 1: Elementary functions")
                .with_layer(1)
                .with_functions(["sin", "cos", "exp", "log", "sqrt", "pow"]),
        );

        self.add_module(
            ModuleInfo::new("linalg", "core.math.linalg")
                .with_description("Layer 2: Linear algebra (BLAS)")
                .with_layer(2)
                .with_types(["Matrix", "Vector"])
                .with_functions(["dot", "matmul", "svd", "eigenvalues"]),
        );

        self.add_module(
            ModuleInfo::new("tensor", "core.math.tensor")
                .with_description("Layer 4: N-dimensional arrays")
                .with_layer(4)
                .with_types(["Tensor", "TensorView", "DType"]),
        );

        self.add_module(
            ModuleInfo::new("gpu", "core.math.gpu")
                .with_description("Layer 5: GPU computing")
                .with_layer(5)
                .with_types(["GpuTensor", "Device", "Kernel"]),
        );

        self.add_module(
            ModuleInfo::new("autodiff", "core.math.autodiff")
                .with_description("Layer 6: Automatic differentiation")
                .with_layer(6)
                .with_functions(["grad", "vjp", "jvp", "jacobian", "hessian"]),
        );

        self.add_module(
            ModuleInfo::new("neural", "core.math.neural")
                .with_description("Layer 7: Neural network primitives")
                .with_layer(7)
                .with_types(["Linear", "Conv2d", "BatchNorm", "Attention", "Embedding"]),
        );

        self.add_module(
            ModuleInfo::new("agents", "core.math.agents")
                .with_description("Layer 8: LLM agents and cognitive systems")
                .with_layer(8)
                .with_types(["Agent", "Tool", "Memory", "Planner"]),
        );

        // Meta layer
        self.add_module(
            ModuleInfo::new("meta", "core.meta")
                .with_description("Meta-programming and reflection")
                .with_types(["Type", "Expr", "TokenStream"])
                .with_functions(["type_of", "size_of", "quote", "unquote"]),
        );
    }

    /// Populate core type documentation
    fn populate_core_types(&mut self) {
        // List<T>
        self.add_doc(
            DocEntry::new("List", "core.collections", DocKind::Type)
                .with_signature("type List<T> is { ... }")
                .with_summary("Growable, heap-allocated sequence of elements")
                .with_description(
                    "List<T> is the primary dynamic array type in Verum. It provides efficient \
                     random access, append, and iteration. Elements are stored contiguously in \
                     heap memory with automatic resizing.",
                )
                .with_examples([
                    "let nums = List::from([1, 2, 3, 4, 5])",
                    "nums.push(6)",
                    "let doubled = nums.map(fn(x) x * 2)",
                    "let sum = nums.reduce(0, fn(a, b) a + b)",
                ])
                .with_see_also(["Map", "Set", "Deque"])
                .with_tags(["collection", "sequence", "dynamic"]),
        );

        // Map<K, V>
        self.add_doc(
            DocEntry::new("Map", "core.collections", DocKind::Type)
                .with_signature("type Map<K: Hash + Eq, V> is { ... }")
                .with_summary("Hash-based key-value mapping")
                .with_description(
                    "Map<K, V> provides O(1) average lookup, insertion, and deletion. \
                     Keys must implement Hash and Eq protocols.",
                )
                .with_examples([
                    "let mut scores = Map::new()",
                    "scores.insert(\"Alice\", 95)",
                    "let alice = scores.get(\"Alice\")",
                ])
                .with_see_also(["BTreeMap", "Set"])
                .with_tags(["collection", "hash", "associative"]),
        );

        // Tensor
        self.add_doc(
            DocEntry::new("Tensor", "core.math.tensor", DocKind::Type)
                .with_signature("type Tensor<T: Numeric, Shape: List<Int>> is { ... }")
                .with_summary("N-dimensional array for numerical computing")
                .with_description(
                    "Tensor is the fundamental type for numerical computing in Verum. \
                     It supports automatic differentiation, GPU acceleration, and \
                     broadcasting operations.",
                )
                .with_examples([
                    "let a = Tensor::from([[1.0, 2.0], [3.0, 4.0]])",
                    "let b = Tensor::randn([2, 2])",
                    "let c = a.matmul(b)",
                    "let grad_fn = grad(fn(x) x.pow(2).sum())",
                ])
                .with_see_also(["GpuTensor", "autodiff.grad"])
                .with_tags(["tensor", "math", "gpu", "autodiff"]),
        );

        // Maybe<T>
        self.add_doc(
            DocEntry::new("Maybe", "core.base", DocKind::Type)
                .with_signature("type Maybe<T> is None | Some(T)")
                .with_summary("Optional value that may or may not be present")
                .with_description(
                    "Maybe<T> represents an optional value. Use it instead of null pointers \
                     for type-safe handling of missing values.",
                )
                .with_examples([
                    "let x: Maybe<Int> = Some(42)",
                    "let y: Maybe<Int> = None",
                    "match x { Some(n) => print(n), None => print(\"missing\") }",
                    "let value = x.unwrap_or(0)",
                ])
                .with_see_also(["Result"])
                .with_tags(["option", "null-safety", "monad"]),
        );

        // Result<T, E>
        self.add_doc(
            DocEntry::new("Result", "core.base", DocKind::Type)
                .with_signature("type Result<T, E> is Ok(T) | Err(E)")
                .with_summary("Represents success or failure with error details")
                .with_description(
                    "Result<T, E> is the standard error handling type. Use Ok for successful \
                     values and Err for error information. The ? operator propagates errors.",
                )
                .with_examples([
                    "fn divide(a: Int, b: Int) -> Result<Int, Text> { ... }",
                    "let result = divide(10, 2)?  // propagates error",
                    "match result { Ok(v) => v, Err(e) => panic(e) }",
                ])
                .with_see_also(["Maybe"])
                .with_tags(["error", "handling", "propagation"]),
        );

        // Iterator protocol
        self.add_doc(
            DocEntry::new("Iterator", "core.base", DocKind::Protocol)
                .with_signature(
                    "type Iterator is protocol {\n    type Item;\n    fn next(&mut self) -> Maybe<Self.Item>;\n}",
                )
                .with_summary("Protocol for lazy sequential access")
                .with_description(
                    "Iterator enables lazy evaluation of sequences. All collections implement \
                     Iterator, and it provides combinators like map, filter, reduce, and take.",
                )
                .with_examples([
                    "[1, 2, 3].iter().map(fn(x) x * 2).collect()",
                    "(0..).take(10).filter(fn(x) x % 2 == 0)",
                ])
                .with_tags(["iterator", "lazy", "protocol"]),
        );

        // grad function
        self.add_doc(
            DocEntry::new("grad", "core.math.autodiff", DocKind::Function)
                .with_signature("fn grad<T>(f: fn(T) -> Tensor) -> fn(T) -> T")
                .with_summary("Compute gradient of a scalar-valued function")
                .with_description(
                    "grad transforms a function returning a scalar into a function returning \
                     the gradient. Uses reverse-mode automatic differentiation (VJP).",
                )
                .with_examples([
                    "let f = fn(x: Tensor) x.pow(2).sum()",
                    "let df = grad(f)",
                    "df(tensor)  // returns 2*tensor",
                ])
                .with_see_also(["vjp", "jvp", "jacobian"])
                .with_tags(["autodiff", "gradient", "derivative"]),
        );
    }

    /// Populate examples
    fn populate_examples(&mut self) {
        for example in super::examples::builtin_examples() {
            self.add_example(example);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_tree() {
        let mut tree = ModuleTree::new();
        tree.add_module(
            ModuleInfo::new("collections", "core.collections")
                .with_description("Collection types"),
        );
        tree.add_module(
            ModuleInfo::new("list", "core.collections.list")
                .with_description("List type"),
        );

        assert!(tree.get("core.collections").is_some());
        assert!(tree.get("core.collections.list").is_some());
    }

    #[test]
    fn test_discovery_index() {
        let index = DiscoveryIndex::standard_library();

        // Check modules exist
        assert!(index.modules.get("core.collections").is_some());
        assert!(index.modules.get("core.math.tensor").is_some());

        // Check types exist
        assert!(index.get_doc("List").is_some());
        assert!(index.get_doc("Tensor").is_some());

        // Check examples exist
        assert!(!index.examples.is_empty());
    }

    #[test]
    fn test_tag_index() {
        let index = DiscoveryIndex::standard_library();
        let collection_items = index.by_tag("collection");
        assert!(!collection_items.is_empty());
    }
}
