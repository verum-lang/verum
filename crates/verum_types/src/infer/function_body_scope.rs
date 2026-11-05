//! Lexical parameter identities shared by ordinary and method body checking.
use super::TypeChecker;
use crate::Result;
use verum_ast::{FunctionDecl, ty::GenericParamKind};
use verum_common::List;

impl TypeChecker {
    pub(super) fn with_function_parameter_scope<T>(
        &mut self,
        function: &FunctionDecl,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let mut saved = List::new();
        for parameter in &function.generics {
            let name = match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::KindAnnotated { name, .. } => &name.name,
                _ => continue,
            };
            saved.push((name.clone(), self.ctx.type_defs.get(name).cloned()));
        }
        let inherited = self.decl_param_frames.last().cloned().unwrap_or_default();
        self.push_decl_param_frame(inherited);
        let result = operation(self);
        self.pop_decl_param_frame();
        for (name, previous) in saved {
            match previous {
                Some(ty) => {
                    self.ctx.type_defs.insert(name, ty);
                }
                None => {
                    self.ctx.type_defs.remove(&name);
                }
            }
        }
        result
    }
}
