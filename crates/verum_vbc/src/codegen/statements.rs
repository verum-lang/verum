//! Statement compilation for VBC codegen.
//!
//! Transforms Verum AST statements into VBC instructions.

use super::{CodegenError, CodegenResult, VbcCodegen};
use crate::instruction::{Instruction, Reg};

use verum_ast::{Stmt, StmtKind};

impl VbcCodegen {
    /// Compiles a statement and returns the result register (if any).
    pub fn compile_stmt(&mut self, stmt: &Stmt) -> CodegenResult<Option<Reg>> {
        // @cfg statement-level filtering
        // Skip statements with non-matching @cfg attributes for the target platform.
        // This prevents issues like:
        // - "return outside function" from returns in skipped @cfg blocks
        // - "undefined variable" from variables defined in @cfg blocks used outside
        // - Platform-specific code generating invalid bytecode
        if !self.should_compile_stmt(stmt) {
            self.ctx.stats.cfg_filtered_stmts += 1;
            return Ok(None);
        }

        self.ctx.stats.statements_compiled += 1;

        // Extract loop and branch hints from statement attributes
        let loop_hints = self.extract_loop_hints_from_attrs(&stmt.attributes);
        let branch_hint = self.extract_branch_hint_from_attrs(&stmt.attributes);

        match &stmt.kind {
            StmtKind::Let { pattern, ty, value } => {
                self.compile_let(pattern, ty.as_ref(), value.as_ref())
            }

            StmtKind::LetElse { pattern, ty: _, value, else_block } => {
                self.compile_let_else(pattern, value, else_block)
            }

            StmtKind::Expr { expr, has_semi } => {
                // Emit loop optimization hints before loop expressions
                if (loop_hints.unroll.is_some() || loop_hints.vectorize.is_some())
                    && matches!(expr.kind, verum_ast::ExprKind::While { .. } | verum_ast::ExprKind::For { .. } | verum_ast::ExprKind::Loop { .. }) {
                        self.ctx.emit(Instruction::LoopHint { hints: loop_hints });
                    }
                // Emit branch hints before if expressions
                if let Some(likely) = branch_hint
                    && matches!(expr.kind, verum_ast::ExprKind::If { .. }) {
                        self.ctx.emit(Instruction::BranchHint { likely });
                    }
                let result = self.compile_expr(expr)?;
                if *has_semi {
                    // Statement expression - discard result
                    if let Some(reg) = result {
                        self.ctx.free_temp(reg);
                    }
                    Ok(None)
                } else {
                    // Trailing expression - return result
                    Ok(result)
                }
            }

            StmtKind::Item(item) => {
                // Handle nested items:
                // - Functions are compiled separately (already registered in collect_declarations)
                //   They should NOT be compiled inline as it would reset the context.
                // - Other items (types, constants) can be compiled inline.
                match &item.kind {
                    verum_ast::ItemKind::Function(func) => {
                        // Check if this nested function captures variables
                        // from the enclosing scope. If so, compile it as a
                        // closure RIGHT HERE — while we're still inside the
                        // outer function's scope where captured variables
                        // are accessible.
                        let has_captures = if let verum_common::Maybe::Some(ref body) = func.body {
                            let param_names: Vec<String> = func.params.iter()
                                .filter_map(|p| {
                                    if let verum_ast::decl::FunctionParamKind::Regular { pattern, .. } = &p.kind
                                        && let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind {
                                            return Some(name.name.to_string());
                                        }
                                    None
                                })
                                .collect();
                            // Wrap body in a block expression for analysis
                            let body_expr = match body {
                                verum_ast::FunctionBody::Block(blk) => {
                                    verum_ast::Expr::new(
                                        verum_ast::ExprKind::Block(blk.clone()),
                                        func.span,
                                    )
                                }
                                verum_ast::FunctionBody::Expr(e) => e.clone(),
                            };
                            let free = self.analyze_free_variables(&body_expr, &param_names);
                            free.iter().any(|v| self.ctx.lookup_var(v).is_some())
                        } else {
                            false
                        };

                        if has_captures && let verum_common::Maybe::Some(ref body) = func.body {
                            {
                                // Build closure params from function params
                                let closure_params: verum_common::List<verum_ast::expr::ClosureParam> =
                                    func.params.iter().filter_map(|p| {
                                        if let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } = &p.kind {
                                            Some(verum_ast::expr::ClosureParam {
                                                pattern: pattern.clone(),
                                                ty: verum_common::Maybe::Some(ty.clone()),
                                                span: p.span,
                                            })
                                        } else {
                                            None
                                        }
                                    }).collect();

                                let body_expr = match body {
                                    verum_ast::FunctionBody::Block(blk) => {
                                        verum_ast::Expr::new(
                                            verum_ast::ExprKind::Block(blk.clone()),
                                            func.span,
                                        )
                                    }
                                    verum_ast::FunctionBody::Expr(e) => e.clone(),
                                };

                                let return_type: Option<&verum_ast::ty::Type> =
                                    match &func.return_type {
                                        verum_common::Maybe::Some(ty) => Some(ty),
                                        verum_common::Maybe::None => None,
                                    };

                                // Compile as closure with capture analysis
                                if let Some(closure_reg) = self.compile_closure(
                                    &closure_params,
                                    &body_expr,
                                    return_type,
                                )? {
                                    let fn_name = func.name.name.to_string();
                                    let name_reg = self.ctx.define_var(&fn_name, false);
                                    self.ctx.emit(crate::instruction::Instruction::Mov {
                                        dst: name_reg,
                                        src: closure_reg,
                                    });
                                }
                            }
                        } else if func.body.is_some() {
                            // Non-capturing nested function: compile as
                            // standalone so the function is emitted at the
                            // module level and accessible by name.
                            //
                            // `compile_function` internally calls
                            // `begin_function`/`end_function`, which clear
                            // the current in_function / instructions /
                            // registers state. Without save/restore around
                            // the call, compiling a nested `fn` clobbers
                            // the outer function's context and the outer's
                            // remaining `return` statements end up emitted
                            // with `in_function == false` → codegen error
                            // "return statement outside of function".
                            //
                            // Mirror the save/restore pattern used by
                            // compile_closure (see expressions.rs:11687).
                            let saved_function = self.ctx.current_function.clone();
                            let saved_instructions = std::mem::take(&mut self.ctx.instructions);
                            let saved_registers = self.ctx.registers.snapshot();
                            let saved_in_function = self.ctx.in_function;
                            let saved_return_type = self.ctx.return_type.clone();
                            let saved_closure_ctx = self.ctx.save_closure_context();

                            self.compile_function(func, None)?;

                            // Restore parent function's compilation context.
                            self.ctx.current_function = saved_function;
                            self.ctx.instructions = saved_instructions;
                            self.ctx.registers.restore_reg(&saved_registers);
                            self.ctx.in_function = saved_in_function;
                            self.ctx.return_type = saved_return_type;
                            self.ctx.restore_closure_context(saved_closure_ctx);
                        }
                        // Function is fully handled here — the post-hoc
                        // compile_nested_functions pass in mod.rs should
                        // NOT recompile it.
                    }
                    _ => {
                        self.compile_item(item)?;
                    }
                }
                Ok(None)
            }

            StmtKind::Defer(expr) => {
                self.compile_defer(expr, false)
            }

            StmtKind::Errdefer(expr) => {
                self.compile_defer(expr, true)
            }

            StmtKind::Provide { context, alias: _, value } => {
                self.compile_provide(context, value)
            }

            StmtKind::ProvideScope { context, alias: _, value, block } => {
                self.compile_provide_scope(context, value, block)
            }

            StmtKind::Empty => {
                // No-op
                Ok(None)
            }
        }
    }

    /// Compiles a let binding.
    fn compile_let(
        &mut self,
        pattern: &verum_ast::Pattern,
        ty: Option<&verum_ast::Type>,
        value: Option<&verum_ast::Expr>,
    ) -> CodegenResult<Option<Reg>> {
        // Infer and track variable type for correct instruction selection
        // This must be done BEFORE compiling the expression to handle chained assignments
        if let Some(var_name) = self.extract_pattern_name(pattern) {
            use crate::codegen::context::VarTypeKind;
            use verum_ast::ty::TypeKind;

            // First, check the explicit type annotation (e.g., `let x: Byte = 255`)
            let annotation_type = ty.and_then(|t| {
                match &t.kind {
                    TypeKind::Path(path) => {
                        if let Some(ident) = path.as_ident() {
                            match ident.name.as_str() {
                                "Byte" | "UInt8" | "u8" => Some(VarTypeKind::Byte),
                                "Int32" | "i32" => Some(VarTypeKind::Int32),
                                "UInt64" | "u64" => Some(VarTypeKind::UInt64),
                                "Float" | "Float64" | "f64" | "Float32" | "f32" => Some(VarTypeKind::Float),
                                "Int" | "Int64" | "i64" => Some(VarTypeKind::Int),
                                "Bool" => Some(VarTypeKind::Bool),
                                "Char" => Some(VarTypeKind::Char),
                                "Text" => Some(VarTypeKind::Text),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            });

            // Then try expression inference
            let expr_type = value.and_then(|expr| self.infer_expr_type_kind(expr)).map(|type_kind| {
                match type_kind {
                    TypeKind::Int => VarTypeKind::Int,
                    TypeKind::Float => VarTypeKind::Float,
                    TypeKind::Bool => VarTypeKind::Bool,
                    TypeKind::Char => VarTypeKind::Char,
                    TypeKind::Text => VarTypeKind::Text,
                    TypeKind::Unit => VarTypeKind::Unit,
                    _ => VarTypeKind::Unknown,
                }
            });

            // Annotation takes priority over expression inference
            let var_type = annotation_type.or(expr_type).unwrap_or(VarTypeKind::Unknown);
            self.ctx.register_variable_type(&var_name, var_type);

            // Track type name for custom protocol dispatch (e.g., Eq, Ord).
            // Priority: explicit type annotation > expression inference.
            // For `let x: Maybe<Int> = Some(42)`, use `Maybe` not `Some`.
            let type_name_from_annotation = ty.and_then(|t| {
                use verum_ast::ty::{TypeKind, PathSegment};

                // Helper to extract type name from a path
                let extract_from_path = |path: &verum_ast::ty::Path| -> Option<String> {
                    if let Some(PathSegment::Name(ident)) = path.segments.first() {
                        let name = ident.name.to_string();
                        // Check if it looks like a type name (starts with uppercase)
                        if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                            Some(name)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                match &t.kind {
                    // Simple path type: Point, Ordering
                    TypeKind::Path(path) => extract_from_path(path),
                    // Generic type: Maybe<Int>, Result<T, E>, List<T>
                    // Extract full type including generics for proper type tracking
                    TypeKind::Generic { base, args } => {
                        if let TypeKind::Path(path) = &base.kind {
                            if let Some(base_name) = extract_from_path(path) {
                                if args.is_empty() {
                                    Some(base_name)
                                } else {
                                    // Build full type string with generic arguments
                                    let arg_strs: Vec<String> = args.iter().filter_map(|arg| {
                                        match arg {
                                            verum_ast::ty::GenericArg::Type(ty) => {
                                                self.extract_type_name(ty)
                                            }
                                            _ => None,
                                        }
                                    }).collect();
                                    if arg_strs.is_empty() {
                                        Some(base_name)
                                    } else {
                                        Some(format!("{}<{}>", base_name, arg_strs.join(", ")))
                                    }
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    // Reference type: &Ipv6Addr, &mut SomeType
                    // Extract the pointee type name so method calls auto-deref correctly
                    TypeKind::Reference { inner, .. }
                    | TypeKind::CheckedReference { inner, .. }
                    | TypeKind::UnsafeReference { inner, .. } => {
                        if let TypeKind::Path(path) = &inner.kind {
                            extract_from_path(path)
                        } else if let TypeKind::Generic { base, .. } = &inner.kind {
                            if let TypeKind::Path(path) = &base.kind {
                                extract_from_path(path)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            });

            // Use annotation type name if available, otherwise infer from expression
            if let Some(type_name) = type_name_from_annotation {
                self.ctx.variable_type_names.insert(var_name.clone(), type_name);
            } else if let Some(expr) = value
                && let Some(type_name) = self.extract_expr_type_name(expr) {
                    self.ctx.variable_type_names.insert(var_name.clone(), type_name);
                }
        }

        // Check if this is a byte array type - if so, use specialized byte array allocation
        // This ensures memory intrinsics like memset/memcpy work correctly
        if let Some(byte_array_size) = self.detect_byte_array_type(ty)
            && let Some(expr) = value {
                // Determine the initialization value
                let init_value = self.get_byte_array_init_value(expr);

                if let Some(init_byte) = init_value {
                    // Allocate byte array with the determined init value
                    let result = self.ctx.alloc_temp();
                    let size_reg = self.ctx.alloc_temp();
                    let init_reg = self.ctx.alloc_temp();

                    // Load size
                    self.ctx.emit(Instruction::LoadI { dst: size_reg, value: byte_array_size as i64 });

                    // Load init value
                    self.ctx.emit(Instruction::LoadI { dst: init_reg, value: init_byte as i64 });

                    // Emit NewByteArray instruction via FfiExtended
                    // Format: dst:reg, size:reg, init:reg
                    let operands = vec![result.0 as u8, size_reg.0 as u8, init_reg.0 as u8];
                    self.ctx.emit(Instruction::FfiExtended {
                        sub_op: 0x49, // NewByteArray sub-opcode (FfiSubOpcode::NewByteArray)
                        operands,
                    });

                    self.ctx.free_temp(size_reg);
                    self.ctx.free_temp(init_reg);

                    // Bind to pattern
                    self.compile_pattern_bind(pattern, result)?;

                    // Mark the variable as a byte array for address computation
                    if let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind {
                        self.ctx.mark_byte_array_var(&name.name);
                    }

                    return Ok(None);
                }

                // Check for array list initialization like [1, 2, 3, 4]
                let list_elements = self.get_byte_array_literal_elements(expr);
                if let Some(elements) = list_elements {
                    // Allocate byte array with zeros, then fill with values
                    let result = self.ctx.alloc_temp();
                    let size_reg = self.ctx.alloc_temp();
                    let init_reg = self.ctx.alloc_temp();

                    // Load size
                    self.ctx.emit(Instruction::LoadI { dst: size_reg, value: byte_array_size as i64 });

                    // Load init value (0)
                    self.ctx.emit(Instruction::LoadI { dst: init_reg, value: 0 });

                    // Emit NewByteArray instruction
                    let operands = vec![result.0 as u8, size_reg.0 as u8, init_reg.0 as u8];
                    self.ctx.emit(Instruction::FfiExtended {
                        sub_op: 0x49, // NewByteArray sub-opcode
                        operands,
                    });

                    self.ctx.free_temp(size_reg);
                    self.ctx.free_temp(init_reg);

                    // Fill in the values using ByteArrayStore
                    for (idx, byte_val) in elements.into_iter().enumerate() {
                        let idx_reg = self.ctx.alloc_temp();
                        let val_reg = self.ctx.alloc_temp();

                        self.ctx.emit(Instruction::LoadI { dst: idx_reg, value: idx as i64 });
                        self.ctx.emit(Instruction::LoadI { dst: val_reg, value: byte_val as i64 });

                        // Emit ByteArrayStore: arr[idx] = val
                        let operands = vec![result.0 as u8, idx_reg.0 as u8, val_reg.0 as u8];
                        self.ctx.emit(Instruction::FfiExtended {
                            sub_op: 0x4C, // ByteArrayStore sub-opcode
                            operands,
                        });

                        self.ctx.free_temp(idx_reg);
                        self.ctx.free_temp(val_reg);
                    }

                    // Bind to pattern
                    self.compile_pattern_bind(pattern, result)?;

                    // Mark the variable as a byte array
                    if let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind {
                        self.ctx.mark_byte_array_var(&name.name);
                    }

                    return Ok(None);
                }

                // Check for repeat syntax with variable value like [value; N]
                // where value is not a literal (e.g., a variable)
                if let Some(value_expr) = self.get_byte_array_repeat_expr(expr) {
                    // Allocate byte array with zeros first
                    let result = self.ctx.alloc_temp();
                    let size_reg = self.ctx.alloc_temp();
                    let init_reg = self.ctx.alloc_temp();

                    // Load size
                    self.ctx.emit(Instruction::LoadI { dst: size_reg, value: byte_array_size as i64 });

                    // Load init value (0 - will be overwritten)
                    self.ctx.emit(Instruction::LoadI { dst: init_reg, value: 0 });

                    // Emit NewByteArray instruction
                    let operands = vec![result.0 as u8, size_reg.0 as u8, init_reg.0 as u8];
                    self.ctx.emit(Instruction::FfiExtended {
                        sub_op: 0x49, // NewByteArray sub-opcode
                        operands,
                    });

                    self.ctx.free_temp(size_reg);
                    self.ctx.free_temp(init_reg);

                    // Compile the value expression
                    let val_reg = self.compile_expr(value_expr)?
                        .ok_or_else(|| CodegenError::internal("byte array repeat value has no result"))?;

                    // Fill all elements with the value (unrolled loop since size is known)
                    for idx in 0..byte_array_size {
                        let idx_reg = self.ctx.alloc_temp();
                        self.ctx.emit(Instruction::LoadI { dst: idx_reg, value: idx as i64 });

                        // Emit ByteArrayStore: arr[idx] = val
                        let operands = vec![result.0 as u8, idx_reg.0 as u8, val_reg.0 as u8];
                        self.ctx.emit(Instruction::FfiExtended {
                            sub_op: 0x4C, // ByteArrayStore sub-opcode
                            operands,
                        });

                        self.ctx.free_temp(idx_reg);
                    }

                    self.ctx.free_temp(val_reg);

                    // Bind to pattern
                    self.compile_pattern_bind(pattern, result)?;

                    // Mark the variable as a byte array
                    if let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind {
                        self.ctx.mark_byte_array_var(&name.name);
                    }

                    return Ok(None);
                }
            }

        // Check if this is a typed array type (non-byte) - if so, use specialized typed array allocation
        // This enables memory intrinsics like memcpy to work correctly with [UInt64; N] arrays
        if let Some((count, elem_size)) = self.detect_typed_array_type(ty)
            && let Some(expr) = value {
                // Check for repeat syntax [value; N] or list syntax [a, b, c, ...]
                let init_value = self.get_typed_array_init_value(expr);

                // Allocate typed array
                let result = self.ctx.alloc_temp();
                let count_reg = self.ctx.alloc_temp();
                let init_reg = self.ctx.alloc_temp();

                // Load count
                self.ctx.emit(Instruction::LoadI { dst: count_reg, value: count as i64 });

                // Load init value (default to 0 if not provided)
                let init_val = init_value.unwrap_or(0);
                self.ctx.emit(Instruction::LoadI { dst: init_reg, value: init_val });

                // Emit NewTypedArray instruction via FfiExtended
                // Format: dst:reg, count:reg, elem_size:u8, init:reg
                let operands = vec![result.0 as u8, count_reg.0 as u8, elem_size as u8, init_reg.0 as u8];
                self.ctx.emit(Instruction::FfiExtended {
                    sub_op: 0x4E, // NewTypedArray sub-opcode (FfiSubOpcode::NewTypedArray)
                    operands,
                });

                self.ctx.free_temp(count_reg);
                self.ctx.free_temp(init_reg);

                // Check if we need to fill with list values
                if let Some(elements) = self.get_typed_array_literal_elements(expr) {
                    // Fill in the values using DerefMutRaw (fast path for all-literal arrays)
                    for (idx, elem_val) in elements.into_iter().enumerate() {
                        // Get element address
                        let idx_reg = self.ctx.alloc_temp();
                        let addr_reg = self.ctx.alloc_temp();
                        let val_reg = self.ctx.alloc_temp();

                        self.ctx.emit(Instruction::LoadI { dst: idx_reg, value: idx as i64 });

                        // TypedArrayElementAddr: addr = &arr[idx] with elem_size
                        let addr_operands = vec![addr_reg.0 as u8, result.0 as u8, idx_reg.0 as u8, elem_size as u8];
                        self.ctx.emit(Instruction::FfiExtended {
                            sub_op: 0x4D, // TypedArrayElementAddr
                            operands: addr_operands,
                        });

                        // Load value
                        self.ctx.emit(Instruction::LoadI { dst: val_reg, value: elem_val });

                        // DerefMutRaw: *addr = val with elem_size
                        let store_operands = vec![addr_reg.0 as u8, val_reg.0 as u8, elem_size as u8];
                        self.ctx.emit(Instruction::FfiExtended {
                            sub_op: 0x61, // DerefMutRaw
                            operands: store_operands,
                        });

                        self.ctx.free_temp(idx_reg);
                        self.ctx.free_temp(addr_reg);
                        self.ctx.free_temp(val_reg);
                    }
                } else if let verum_ast::ExprKind::Array(verum_ast::ArrayExpr::List(elements)) = &expr.kind {
                    // Slow path: compile each element expression and store at runtime
                    for (idx, elem_expr) in elements.iter().enumerate() {
                        if let Some(val_reg) = self.compile_expr(elem_expr)? {
                            let idx_reg = self.ctx.alloc_temp();
                            let addr_reg = self.ctx.alloc_temp();

                            self.ctx.emit(Instruction::LoadI { dst: idx_reg, value: idx as i64 });

                            // TypedArrayElementAddr: addr = &arr[idx] with elem_size
                            let addr_operands = vec![addr_reg.0 as u8, result.0 as u8, idx_reg.0 as u8, elem_size as u8];
                            self.ctx.emit(Instruction::FfiExtended {
                                sub_op: 0x4D, // TypedArrayElementAddr
                                operands: addr_operands,
                            });

                            // DerefMutRaw: *addr = val with elem_size
                            let store_operands = vec![addr_reg.0 as u8, val_reg.0 as u8, elem_size as u8];
                            self.ctx.emit(Instruction::FfiExtended {
                                sub_op: 0x61, // DerefMutRaw
                                operands: store_operands,
                            });

                            self.ctx.free_temp(idx_reg);
                            self.ctx.free_temp(addr_reg);
                            self.ctx.free_temp(val_reg);
                        }
                    }
                }

                // Bind to pattern
                self.compile_pattern_bind(pattern, result)?;

                // Mark the variable as a typed array
                if let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind {
                    self.ctx.mark_typed_array_var(&name.name, elem_size);
                }

                return Ok(None);
            }

        // Compile initializer if present.
        //
        // Variant-name disambiguation: push the let-binding's type
        // annotation as `current_return_type_name` for the duration of the
        // initializer compile so that bare variant constructors in the RHS
        // (e.g. `let x: Maybe<Int> = None;`) resolve to the annotation's
        // type — without this, `find_function_by_suffix(".None")` sees
        // multiple registered variants named `None` across stdlib
        // (`Maybe`, `core.net.tls`, `core.term.widget.block`,
        // `core.mesh.xds.types`, ...) and returns nondeterministically.
        let saved_return_type = if let Some(ty) = ty {
            self.extract_base_type_name(ty).map(|base| {
                let saved = self.ctx.current_return_type_name.clone();
                self.ctx.current_return_type_name = Some(base);
                saved
            })
        } else {
            None
        };
        let init_reg = if let Some(expr) = value {
            self.compile_expr(expr)?
        } else {
            // No initializer - use unit
            let reg = self.ctx.alloc_temp();
            self.ctx.emit(Instruction::LoadUnit { dst: reg });
            Some(reg)
        };
        if let Some(saved) = saved_return_type {
            self.ctx.current_return_type_name = saved;
        }

        // Bind pattern
        if let Some(reg) = init_reg {
            self.compile_pattern_bind(pattern, reg)?;
        }

        // Late initialization: mark variable as uninitialized when no initializer
        if value.is_none()
            && let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind
                && let Some(info) = self.ctx.lookup_var_mut(&name.name) {
                    info.is_initialized = false;
                }

        Ok(None)
    }

    /// Detects if type annotation is a byte array [Byte; N] and returns the size.
    fn detect_byte_array_type(&self, ty: Option<&verum_ast::Type>) -> Option<usize> {
        use verum_ast::ty::{TypeKind, PathSegment};
        use verum_ast::literal::LiteralKind;

        let ty = ty?;
        if let TypeKind::Array { element, size } = &ty.kind {
            // Check if element type is Byte/U8
            if let TypeKind::Path(path) = &element.kind
                && let Some(PathSegment::Name(ident)) = path.segments.last() {
                    let name = ident.as_str();
                    if name == "Byte" || name == "U8" || name == "u8" {
                        // Extract size from expression
                        if let Some(size_expr) = size
                            && let verum_ast::ExprKind::Literal(lit) = &size_expr.kind
                                && let LiteralKind::Int(int_lit) = &lit.kind {
                                    return Some(int_lit.value as usize);
                                }
                    }
                }
        }
        None
    }

    /// Detects if type annotation is a typed array [T; N] (non-byte).
    /// Returns (count, element_size) if detected.
    fn detect_typed_array_type(&self, ty: Option<&verum_ast::Type>) -> Option<(usize, usize)> {
        use verum_ast::ty::{TypeKind, PathSegment};
        use verum_ast::literal::LiteralKind;

        let ty = ty?;
        if let TypeKind::Array { element, size } = &ty.kind {
            // Determine element size from element type.
            // The fast parser produces primitive TypeKind variants (Int, Float, Bool, Char)
            // for built-in types, but TypeKind::Path for non-primitive types (Byte, UInt64, etc.).
            let elem_size = match &element.kind {
                // Primitive TypeKind variants produced by the fast parser
                TypeKind::Int => Some(8),
                TypeKind::Float => Some(8),
                TypeKind::Bool => Some(1),
                TypeKind::Char => Some(4),

                // Path-based type names (non-primitive types)
                TypeKind::Path(path) => {
                    if let Some(PathSegment::Name(ident)) = path.segments.last() {
                        let name = ident.as_str();
                        match name {
                            // 1-byte types (handled by detect_byte_array_type)
                            "Byte" | "U8" | "u8" | "Int8" | "I8" | "i8" => None,

                            // 2-byte types
                            "Int16" | "I16" | "i16" | "UInt16" | "U16" | "u16" => Some(2),

                            // 4-byte types
                            "Int32" | "I32" | "i32" | "UInt32" | "U32" | "u32" | "Float32" | "F32" | "f32" => Some(4),

                            // 8-byte types
                            "Int64" | "I64" | "i64" | "UInt64" | "U64" | "u64" |
                            "Int" | "UInt" | "Float64" | "F64" | "f64" | "Float" => Some(8),

                            // Unknown type - don't track as typed array
                            _ => None,
                        }
                    } else {
                        None
                    }
                }

                _ => None,
            };

            if let Some(elem_size) = elem_size {
                // Bool arrays are 1-byte and handled by detect_byte_array_type
                if elem_size == 1 {
                    return None;
                }

                // Extract count from size expression
                if let Some(size_expr) = size
                    && let verum_ast::ExprKind::Literal(lit) = &size_expr.kind
                        && let LiteralKind::Int(int_lit) = &lit.kind {
                            return Some((int_lit.value as usize, elem_size));
                        }
            }
        }
        None
    }

    /// Gets the init value for typed array from repeat syntax [value; N].
    /// Returns Some(value) for literal integers, None otherwise.
    fn get_typed_array_init_value(&self, expr: &verum_ast::Expr) -> Option<i64> {
        use verum_ast::ExprKind;
        use verum_ast::ty::PathSegment;
        use verum_ast::literal::LiteralKind;

        // Check for uninit() or zeroed()
        if let ExprKind::Call { func, .. } = &expr.kind
            && let ExprKind::Path(path) = &func.kind
                && let Some(PathSegment::Name(ident)) = path.segments.last() {
                    let name = ident.as_str();
                    if name == "uninit" || name == "zeroed" {
                        return Some(0);
                    }
                }

        // Check for [value; N] repeat syntax
        if let ExprKind::Array(verum_ast::ArrayExpr::Repeat { value, .. }) = &expr.kind
            && let ExprKind::Literal(lit) = &value.kind
                && let LiteralKind::Int(int_lit) = &lit.kind {
                    return Some(int_lit.value as i64);
                }

        None
    }

    /// Gets the literal values from a typed array list expression like [0x01020304, 0x05060708, ...].
    /// Returns Some(Vec<i64>) if all elements are integer literals, None otherwise.
    fn get_typed_array_literal_elements(&self, expr: &verum_ast::Expr) -> Option<Vec<i64>> {
        use verum_ast::ExprKind;
        use verum_ast::literal::LiteralKind;

        if let ExprKind::Array(verum_ast::ArrayExpr::List(elements)) = &expr.kind {
            let mut values = Vec::new();
            for elem in elements.iter() {
                match &elem.kind {
                    ExprKind::Literal(lit) => {
                        if let LiteralKind::Int(int_lit) = &lit.kind {
                            values.push(int_lit.value as i64);
                        } else {
                            return None; // Non-integer literal
                        }
                    }
                    ExprKind::Unary { op: verum_ast::UnOp::Neg, expr: inner } => {
                        // Handle negative literals: -N
                        if let ExprKind::Literal(lit) = &inner.kind {
                            if let LiteralKind::Int(int_lit) = &lit.kind {
                                values.push(-(int_lit.value as i64));
                            } else {
                                return None;
                            }
                        } else {
                            return None;
                        }
                    }
                    _ => return None, // Non-literal element
                }
            }
            return Some(values);
        }
        None
    }

    /// Checks if expression is a call to uninit() or zeroed().
    #[allow(dead_code)]  // Reserved for future MaybeUninit support
    fn is_uninit_or_zeroed_call(&self, expr: &verum_ast::Expr) -> bool {
        use verum_ast::ExprKind;
        use verum_ast::ty::PathSegment;

        if let ExprKind::Call { func, .. } = &expr.kind
            && let ExprKind::Path(path) = &func.kind
                && let Some(PathSegment::Name(ident)) = path.segments.last() {
                    let name = ident.as_str();
                    return name == "uninit" || name == "zeroed";
                }
        false
    }

    /// Gets the init value for byte array from various initialization patterns.
    /// Returns Some(byte_value) for:
    /// - uninit() -> 0
    /// - zeroed() -> 0
    /// - [value; N] -> value (if value is a literal integer)
    fn get_byte_array_init_value(&self, expr: &verum_ast::Expr) -> Option<u8> {
        use verum_ast::ExprKind;
        use verum_ast::ty::PathSegment;
        use verum_ast::literal::LiteralKind;

        // Check for uninit() or zeroed()
        if let ExprKind::Call { func, .. } = &expr.kind
            && let ExprKind::Path(path) = &func.kind
                && let Some(PathSegment::Name(ident)) = path.segments.last() {
                    let name = ident.as_str();
                    if name == "uninit" || name == "zeroed" {
                        return Some(0);
                    }
                }

        // Check for [value; N] repeat syntax
        if let ExprKind::Array(verum_ast::ArrayExpr::Repeat { value, .. }) = &expr.kind {
            // Check if value is a literal integer
            if let ExprKind::Literal(lit) = &value.kind
                && let LiteralKind::Int(int_lit) = &lit.kind {
                    // Truncate to u8
                    return Some((int_lit.value & 0xFF) as u8);
                }
            // Check if value is a path to a variable (like `let value: Byte = 42; [value; 4]`)
            // For now, only support literal values
        }

        None
    }

    /// Gets the literal byte values from an array list expression like [1, 2, 3, 4].
    /// Returns Some(Vec<u8>) if all elements are integer literals, None otherwise.
    fn get_byte_array_literal_elements(&self, expr: &verum_ast::Expr) -> Option<Vec<u8>> {
        use verum_ast::ExprKind;
        use verum_ast::literal::LiteralKind;

        if let ExprKind::Array(verum_ast::ArrayExpr::List(elements)) = &expr.kind {
            let mut bytes = Vec::new();
            for elem in elements.iter() {
                if let ExprKind::Literal(lit) = &elem.kind
                    && let LiteralKind::Int(int_lit) = &lit.kind {
                        bytes.push((int_lit.value & 0xFF) as u8);
                        continue;
                    }
                // Non-literal element, can't optimize
                return None;
            }
            return Some(bytes);
        }

        None
    }

    /// Gets the value expression from repeat syntax [value; N] when value is NOT a literal.
    /// Returns Some(expr) for variable repeat patterns, None for literals or non-repeat patterns.
    fn get_byte_array_repeat_expr<'a>(&self, expr: &'a verum_ast::Expr) -> Option<&'a verum_ast::Expr> {
        use verum_ast::ExprKind;
        use verum_ast::literal::LiteralKind;

        if let ExprKind::Array(verum_ast::ArrayExpr::Repeat { value, .. }) = &expr.kind {
            // Only return if value is NOT a literal (literals are handled elsewhere)
            if let ExprKind::Literal(lit) = &value.kind
                && matches!(&lit.kind, LiteralKind::Int(_)) {
                    // This is a literal - handled by get_byte_array_init_value
                    return None;
                }
            // Value is a variable or other expression
            return Some(value.as_ref());
        }

        None
    }

    /// Compiles a let-else binding.
    fn compile_let_else(
        &mut self,
        pattern: &verum_ast::Pattern,
        value: &verum_ast::Expr,
        else_block: &verum_ast::Block,
    ) -> CodegenResult<Option<Reg>> {
        // Compile value
        let value_reg = self.compile_expr(value)?
            .ok_or_else(|| CodegenError::internal("let-else value has no value"))?;

        // Test pattern match
        let match_reg = self.ctx.alloc_temp();
        self.compile_pattern_test(pattern, value_reg, match_reg)?;

        let bind_label = self.ctx.new_label("let_else_bind");

        // If matches, jump to binding
        self.ctx.emit_forward_jump(&bind_label, |offset| {
            Instruction::JmpIf { cond: match_reg, offset }
        });
        self.ctx.free_temp(match_reg);

        // Else block (must diverge)
        self.compile_block(else_block)?;
        // Note: else block should return/break/continue/panic, so no fallthrough

        // Bind pattern
        self.ctx.define_label(&bind_label);
        self.compile_pattern_bind(pattern, value_reg)?;

        Ok(None)
    }

    /// Compiles a defer statement.
    fn compile_defer(
        &mut self,
        expr: &verum_ast::Expr,
        is_errdefer: bool,
    ) -> CodegenResult<Option<Reg>> {
        // Compile the expression to instructions, but don't emit yet
        // Save the instructions to be emitted on scope exit

        // For now, compile to a temp buffer
        let saved_instrs = std::mem::take(&mut self.ctx.instructions);

        // Compile the deferred expression
        self.compile_expr(expr)?;

        // Capture the generated instructions
        let defer_instrs = std::mem::replace(&mut self.ctx.instructions, saved_instrs);

        // Add to defer stack
        self.ctx.add_defer(defer_instrs, is_errdefer);

        Ok(None)
    }

    /// Compiles a provide statement.
    ///
    /// Handles two cases:
    /// 1. Normal provide: `provide Ctx = expr;` — emits a single CtxProvide
    /// 2. Layer expansion: `provide LayerName;` — expands the layer into individual provides
    fn compile_provide(
        &mut self,
        context: &str,
        value: &verum_ast::Expr,
    ) -> CodegenResult<Option<Reg>> {
        // Check if this is a layer expansion (provide LayerName;)
        // Layer provides use an empty tuple () as sentinel value expression
        if let verum_ast::ExprKind::Tuple(elems) = &value.kind
            && elems.is_empty()
                && let Some(provides) = self.resolve_layer_provides(context) {
                    // Expand layer into individual provides
                    for (ctx_name, expr) in provides {
                        let value_reg = self.compile_expr(&expr)?
                            .ok_or_else(|| CodegenError::internal("layer provide value has no value"))?;
                        let ctx_id = self.intern_string(&ctx_name);
                        self.ctx.emit(Instruction::CtxProvide {
                            ctx_type: ctx_id,
                            value: value_reg,
                            body_offset: 0,
                        });
                    }
                    return Ok(None);
                }

        // Normal provide: compile value and emit single CtxProvide
        let value_reg = self.compile_expr(value)?
            .ok_or_else(|| CodegenError::internal("provide value has no value"))?;

        let context_id = self.intern_string(context);

        // Emit context provide instruction with 0 offset (no scoped body)
        self.ctx.emit(Instruction::CtxProvide {
            ctx_type: context_id,
            value: value_reg,
            body_offset: 0, // No scoped body - context provided for rest of function
        });

        // Don't free value_reg - context owns it now

        Ok(None)
    }

    /// Resolves a layer name into its flattened list of (context_name, value_expr) pairs.
    /// Handles both inline layers and composite layers (recursively flattening).
    fn resolve_layer_provides(&self, layer_name: &str) -> Option<Vec<(String, verum_ast::expr::Expr)>> {
        let mut visited = std::collections::HashSet::new();
        self.resolve_layer_provides_inner(layer_name, &mut visited)
    }

    fn resolve_layer_provides_inner(
        &self,
        layer_name: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> Option<Vec<(String, verum_ast::expr::Expr)>> {
        if !visited.insert(layer_name.to_string()) {
            // Cycle detected — skip to prevent stack overflow
            return None;
        }
        let layer = self.context_layers.get(layer_name)?;
        match layer.clone() {
            super::ContextLayer::Inline(entries) => Some(entries),
            super::ContextLayer::Composite(sub_layers) => {
                let mut all_provides = Vec::new();
                for sub_name in &sub_layers {
                    if let Some(sub_provides) = self.resolve_layer_provides_inner(sub_name, visited) {
                        all_provides.extend(sub_provides);
                    }
                    // If sub-layer not found, silently skip (will be caught by type checker)
                }
                Some(all_provides)
            }
        }
    }

    /// Compiles a provide-scope statement.
    fn compile_provide_scope(
        &mut self,
        context: &str,
        value: &verum_ast::Expr,
        body: &verum_ast::Expr,
    ) -> CodegenResult<Option<Reg>> {
        // Compile value
        let value_reg = self.compile_expr(value)?
            .ok_or_else(|| CodegenError::internal("provide-scope value has no value"))?;

        let context_id = self.intern_string(context);

        // Create end label for body offset calculation
        let end_label = self.ctx.new_label("ctx_end");

        // Enter context scope - we'll patch the body_offset later
        self.ctx.emit_forward_context_provide(&end_label, context_id, value_reg);

        // Compile body (can be block expression or any expression)
        self.ctx.enter_scope();
        let result = self.compile_expr(body)?;
        let (_, defers) = self.ctx.exit_scope(false);

        // Emit defers
        for defer_instrs in defers {
            for instr in defer_instrs {
                self.ctx.emit(instr);
            }
        }

        // End context scope - define label for offset patching
        self.ctx.define_label(&end_label);
        self.ctx.emit(Instruction::CtxEnd);

        Ok(result)
    }

    /// Extract loop optimization hints (@unroll, @vectorize, @no_unroll, @no_vectorize, @simd)
    /// from statement attributes.
    fn extract_loop_hints_from_attrs(&self, attrs: &[verum_ast::decl::Attribute]) -> crate::module::LoopHints {
        use crate::module::{LoopHints, LoopUnrollHint, VectorizeHint};
        let mut hints = LoopHints::default();
        for attr in attrs {
            match attr.name.as_str() {
                "unroll" => {
                    hints.unroll = Some(match &attr.args {
                        verum_common::Maybe::Some(args) if !args.is_empty() => {
                            if let Some(first) = args.first() {
                                match Self::attr_arg_as_ident(first).as_deref() {
                                    Some("full") => LoopUnrollHint::Full,
                                    _ => {
                                        // Try to parse as integer
                                        if let verum_ast::ExprKind::Literal(lit) = &first.kind {
                                            if let verum_ast::LiteralKind::Int(int_lit) = &lit.kind {
                                                LoopUnrollHint::Count(int_lit.value as u32)
                                            } else {
                                                LoopUnrollHint::Full
                                            }
                                        } else {
                                            LoopUnrollHint::Full
                                        }
                                    }
                                }
                            } else {
                                LoopUnrollHint::Full
                            }
                        }
                        _ => LoopUnrollHint::Full,
                    });
                }
                "no_unroll" => {
                    hints.unroll = Some(LoopUnrollHint::Disable);
                }
                "vectorize" | "simd" => {
                    hints.vectorize = Some(match &attr.args {
                        verum_common::Maybe::Some(args) if !args.is_empty() => {
                            if let Some(first) = args.first() {
                                match Self::attr_arg_as_ident(first).as_deref() {
                                    Some("force") => VectorizeHint::Force,
                                    Some("never") => VectorizeHint::Disable,
                                    _ => {
                                        if let verum_ast::ExprKind::Literal(lit) = &first.kind {
                                            if let verum_ast::LiteralKind::Int(int_lit) = &lit.kind {
                                                VectorizeHint::Width(int_lit.value as u32)
                                            } else {
                                                VectorizeHint::Enable
                                            }
                                        } else {
                                            VectorizeHint::Enable
                                        }
                                    }
                                }
                            } else {
                                VectorizeHint::Enable
                            }
                        }
                        _ => VectorizeHint::Enable,
                    });
                }
                "no_vectorize" => {
                    hints.vectorize = Some(VectorizeHint::Disable);
                }
                _ => {}
            }
        }
        hints
    }

    /// Extract branch likelihood hint (@likely, @unlikely) from statement attributes.
    fn extract_branch_hint_from_attrs(&self, attrs: &[verum_ast::decl::Attribute]) -> Option<bool> {
        for attr in attrs {
            match attr.name.as_str() {
                "likely" => return Some(true),
                "unlikely" => return Some(false),
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_statements_module_exists() {
        // Canary: module compiles and links. An empty body is enough —
        // if this file failed to compile, the test binary wouldn't link.
    }
}
