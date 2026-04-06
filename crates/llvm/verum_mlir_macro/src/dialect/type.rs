use verum_tblgen::record::Record;

#[derive(Debug, Clone, Copy)]
pub struct Type {
    optional: bool,
    variadic: bool,
    variadic_of_variadic: bool,
}

impl Type {
    pub fn new(record: Record) -> Self {
        Self {
            optional: record.subclass_of("Optional"),
            variadic: record.subclass_of("Variadic"),
            variadic_of_variadic: record.subclass_of("VariadicOfVariadic"),
        }
    }

    pub const fn is_optional(&self) -> bool {
        self.optional
    }

    pub const fn is_variadic(&self) -> bool {
        self.variadic
    }

    /// Variadic-of-variadic is parsed from ODS but not yet used in code generation.
    #[allow(unused)]
    pub const fn is_variadic_of_variadic(&self) -> bool {
        self.variadic_of_variadic
    }

    pub const fn is_unfixed(&self) -> bool {
        self.is_variadic() || self.is_optional()
    }
}
