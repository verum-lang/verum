//! Effective usage constraints follow exact descriptors and owned components.
//! This query supplies no duplication operation or cleanup obligation.

use crate::module::VbcModule;
use crate::mono::TypeSubstitution;
use crate::types::{TypeId, TypeKind, TypeRef};
use verum_common::{List, Map, ResourceDiscipline};

impl VbcModule {
    /// Resolve usage discipline from exact declarations and substituted owned fields.
    /// Unknown components or missing/legacy descriptors never imply unrestricted use.
    pub fn resource_discipline(&self, ty: &TypeRef) -> ResourceDiscipline {
        self.resource_discipline_inner(ty, &mut List::new(), &mut Map::new(), &mut 4096, 0)
    }

    fn resource_discipline_inner(
        &self,
        ty: &TypeRef,
        active: &mut List<TypeRef>,
        memo: &mut Map<TypeRef, ResourceDiscipline>,
        remaining: &mut usize,
        depth: usize,
    ) -> ResourceDiscipline {
        use ResourceDiscipline::{Unknown, Unrestricted};
        if let Some(mode) = memo.get(ty) {
            return *mode;
        }
        if depth >= 128 || *remaining == 0 {
            return Unknown;
        }
        *remaining -= 1;
        let (id, args) = match ty {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            // Copying a reference does not consume the referenced resource.
            TypeRef::Reference { .. } | TypeRef::Slice(_) => return Unrestricted,
            TypeRef::Tuple(parts) => {
                return parts.iter().fold(Unrestricted, |mode, part| {
                    mode.with_component(self.resource_discipline_inner(
                        part,
                        active,
                        memo,
                        remaining,
                        depth + 1,
                    ))
                });
            }
            TypeRef::Array { element, .. } => {
                return self.resource_discipline_inner(element, active, memo, remaining, depth + 1);
            }
            TypeRef::Generic(_)
            | TypeRef::AssociatedProjection { .. }
            | TypeRef::Function { .. }
            | TypeRef::Rank2Function { .. }
            | TypeRef::ConstValue(_) => return Unknown,
        };
        let Some(descriptor) = self.get_type(id) else {
            // PTR is also the legacy unresolved-type placeholder. Its numeric
            // carrier cannot distinguish a proven raw view from a lost owner.
            return if id.is_builtin() && id != TypeId::PTR && id != TypeId::RESERVED {
                Unrestricted
            } else {
                Unknown
            };
        };
        let mut mode = descriptor.resource_discipline;
        if mode == Unknown || active.contains(ty) {
            return Unknown;
        }
        if descriptor.type_params.len() != args.len() {
            return Unknown;
        }
        if matches!(descriptor.kind, TypeKind::Protocol | TypeKind::Tensor) {
            return Unknown;
        }
        active.push(ty.clone());
        let substitution = TypeSubstitution::new(&descriptor.type_params, args);
        if descriptor.kind == TypeKind::Alias {
            mode = descriptor.alias_target.as_ref().map_or(Unknown, |target| {
                mode.with_component(self.resource_discipline_inner(
                    &substitution.apply(target),
                    active,
                    memo,
                    remaining,
                    depth + 1,
                ))
            });
        } else {
            for field in &descriptor.fields {
                mode = mode.with_component(field.declaration_type.as_ref().map_or(Unknown, |ty| {
                    self.resource_discipline_inner(
                        &substitution.apply(ty),
                        active,
                        memo,
                        remaining,
                        depth + 1,
                    )
                }));
            }
            for variant in &descriptor.variants {
                if let Some(payload) = &variant.payload {
                    mode = mode.with_component(self.resource_discipline_inner(
                        &substitution.apply(payload),
                        active,
                        memo,
                        remaining,
                        depth + 1,
                    ));
                }
                for field in &variant.fields {
                    mode = mode.with_component(field.declaration_type.as_ref().map_or(
                        Unknown,
                        |ty| {
                            self.resource_discipline_inner(
                                &substitution.apply(ty),
                                active,
                                memo,
                                remaining,
                                depth + 1,
                            )
                        },
                    ));
                }
            }
        }
        active.pop();
        memo.insert(ty.clone(), mode);
        mode
    }
}
