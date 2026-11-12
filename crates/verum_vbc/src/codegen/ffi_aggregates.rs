//! Native products share one layout; the retained carrier distinguishes
//! a record's slots from a fixed array's element storage.
use super::*;
use verum_common::List;

impl VbcCodegen {
    pub(super) fn ffi_field_layout(
        &mut self,
        ty: &verum_ast::ty::Type,
        siblings: &Map<Text, List<verum_ast::decl::RecordField>>,
        active: &mut List<Text>,
        array_depth: usize,
    ) -> CodegenResult<(CType, u16, u16, Option<u16>)> {
        use verum_ast::ty::TypeKind;
        if active.len() + array_depth >= 8 {
            return Err(CodegenError::unsupported_expr(
                "native aggregate nesting exceeds 8 levels",
            ));
        }
        if let TypeKind::Array { element, size } = &ty.kind {
            let count = size
                .as_deref()
                .map(|size| self.const_eval_i64(size))
                .transpose()?
                .flatten()
                .filter(|count| *count > 0)
                .ok_or_else(|| {
                    CodegenError::unsupported_expr(
                        "native fixed-array fields require a positive resolved extent",
                    )
                })?;
            let (element_type, width, align, nested) =
                self.ffi_field_layout(element, siblings, active, array_depth + 1)?;
            let bytes = count
                .checked_mul(i64::from(width))
                .and_then(|size| u16::try_from(size).ok())
                .filter(|size| *size != 0)
                .ok_or_else(|| {
                    CodegenError::unsupported_expr(
                        "native fixed-array field exceeds the layout size limit",
                    )
                })?;
            let name = format!("<array:{element_type:?}:{width}:{nested:?};{count}>");
            let index = if let Some(index) = self.repr_c_types.get(&name) {
                *index
            } else {
                let index = u16::try_from(self.ffi_layouts.len()).map_err(|_| {
                    CodegenError::unsupported_expr("too many native aggregate layouts")
                })?;
                let name_id = crate::types::StringId(self.intern_string(&name));
                let mut layout = FfiStructLayout::new(name_id, u32::from(bytes), align);
                layout.verum_type = Some(crate::types::TypeId::ARRAY);
                for element in 0..count as u32 {
                    layout.fields.push(FfiStructField {
                        name: crate::types::StringId(self.intern_string(&element.to_string())),
                        c_type: element_type,
                        offset: element * u32::from(width),
                        size: width,
                        align,
                        nested_layout: nested,
                    });
                }
                self.ffi_layouts.push(layout);
                self.repr_c_types.insert(name, index);
                index
            };
            return Ok((CType::StructValue, bytes, align, Some(index)));
        }
        if let Some(name) = Self::ffi_referenced_struct_name(ty)
            && !self.repr_c_types.contains_key(&name)
        {
            if active.iter().any(|entry| entry.as_str() == name)
                && matches!(ty.kind, TypeKind::Path(_))
            {
                return Err(CodegenError::unsupported_expr(
                    "recursive by-value native record",
                ));
            }
            if !active.iter().any(|entry| entry.as_str() == name)
                && let Some(fields) = siblings.get(&Text::from(name.as_str())).cloned()
            {
                self.generate_ffi_struct_layout_inner(&name, &fields, siblings, active)?;
            }
        }
        let c_type = self.verum_type_to_ctype(&Some(ty.clone()));
        let nested = self.ffi_nested_layout_index(c_type, ty);
        let (size, align) = if c_type == CType::StructValue {
            let layout = nested
                .and_then(|index| self.ffi_layouts.get(index as usize))
                .ok_or_else(|| {
                    CodegenError::unsupported_expr("native record field has no declared layout")
                })?;
            (
                u16::try_from(layout.size).map_err(|_| {
                    CodegenError::unsupported_expr(
                        "native record field exceeds the layout size limit",
                    )
                })?,
                layout.align,
            )
        } else {
            self.ctype_size_align(c_type)
        };
        if size == 0 {
            return Err(CodegenError::unsupported_expr(
                "native record field has no resolved storage layout",
            ));
        }
        Ok((c_type, size, align, nested))
    }

    pub(super) fn validate_ffi_layout_depth(&self, root: u16) -> CodegenResult<()> {
        let mut pending = List::new();
        let mut deepest = Map::new();
        pending.push((root, 0));
        while let Some((index, depth)) = pending.pop() {
            if depth >= 8 {
                return Err(CodegenError::unsupported_expr(
                    "native aggregate nesting exceeds 8 levels",
                ));
            }
            if deepest.get(&index).is_some_and(|seen| *seen >= depth) {
                continue;
            }
            deepest.insert(index, depth);
            for field in &self.ffi_layouts[index as usize].fields {
                if let Some(child) = field.nested_layout {
                    pending.push((child, depth + 1));
                }
            }
        }
        Ok(())
    }
}
