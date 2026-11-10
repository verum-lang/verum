//! Relocate foreign-call descriptors and the bytecode operands referring to them.
use super::{LinkError, RemapTable};
use crate::{
    encoding::{decode_reg, decode_u16, decode_u32},
    instruction::{Instruction, SystemSubOpcode},
    module::VbcModule,
};

#[derive(Default, Debug)]
struct Table {
    first: u32,
    count: u32,
}

impl Table {
    fn new(
        first: usize,
        count: usize,
        maximum: u32,
        kind: &'static str,
    ) -> Result<Self, LinkError> {
        let invalid = || LinkError::DanglingReference {
            kind,
            source_id: u32::MAX,
        };
        let first = u32::try_from(first).map_err(|_| invalid())?;
        let count = u32::try_from(count).map_err(|_| invalid())?;
        if count != 0
            && first
                .checked_add(count - 1)
                .is_none_or(|last| last > maximum)
        {
            return Err(invalid());
        }
        Ok(Self { first, count })
    }

    fn map(&self, id: u32, kind: &'static str) -> Result<u32, LinkError> {
        if id >= self.count {
            return Err(LinkError::DanglingReference {
                kind,
                source_id: id,
            });
        }
        Ok(self.first + id)
    }
}

#[derive(Default, Debug)]
pub(super) struct ForeignTables {
    libraries: Table,
    symbols: Table,
    layouts: Table,
}

impl ForeignTables {
    pub(super) fn new(out: &VbcModule, source: &VbcModule) -> Result<Self, LinkError> {
        Ok(Self {
            libraries: Table::new(
                out.ffi_libraries.len(),
                source.ffi_libraries.len(),
                i16::MAX as u32,
                "FFI library table",
            )?,
            symbols: Table::new(
                out.ffi_symbols.len(),
                source.ffi_symbols.len(),
                u32::MAX,
                "FFI symbol table",
            )?,
            layouts: Table::new(
                out.ffi_layouts.len(),
                source.ffi_layouts.len(),
                u16::MAX as u32,
                "FFI layout table",
            )?,
        })
    }

    fn layout(&self, id: u16) -> Result<u16, LinkError> {
        self.layouts
            .map(u32::from(id), "FFI layout")
            .map(|mapped| mapped as u16)
    }

    pub(super) fn relocate_instruction(
        &self,
        instruction: &mut Instruction,
    ) -> Result<(), LinkError> {
        let Instruction::FfiExtended { sub_op, operands } = instruction else {
            return Ok(());
        };
        use SystemSubOpcode::*;
        let (registers, skip, table, kind, width) = match SystemSubOpcode::from_byte(*sub_op) {
            Some(
                CallFfiC | CallFfiStdcall | CallFfiSysV64 | CallFfiFastcall | CallFfiVariadic
                | CallFfiAarch64 | CallFfiWin64Arm64,
            ) => (0, 0, &self.symbols, "FFI symbol", 4),
            Some(LoadSymbol | IsSymbolResolved | CallFfiIndirect) => {
                (1, 0, &self.symbols, "FFI symbol", 4)
            }
            Some(CreateCallback) => (1, 4, &self.symbols, "FFI callback signature", 4),
            Some(GetLibrary) => (1, 0, &self.libraries, "FFI library", 2),
            Some(StructToC | StructFromC) => (2, 0, &self.layouts, "FFI layout", 4),
            _ => return Ok(()),
        };
        let mut offset = 0;
        let malformed = || LinkError::TruncatedBytecode {
            offset: 0,
            want: operands.len(),
        };
        for _ in 0..registers {
            decode_reg(operands, &mut offset).map_err(|_| malformed())?;
        }
        offset += skip;
        let start = offset;
        let source = if width == 2 {
            u32::from(decode_u16(operands, &mut offset).map_err(|_| malformed())?)
        } else {
            decode_u32(operands, &mut offset).map_err(|_| malformed())?
        };
        let mapped = table.map(source, kind)?.to_le_bytes();
        operands[start..offset].copy_from_slice(&mapped[..width]);
        Ok(())
    }
}

pub(super) fn append(
    out: &mut VbcModule,
    source: &VbcModule,
    remap: &RemapTable,
) -> Result<(), LinkError> {
    for library in &source.ffi_libraries {
        let mut library = library.clone();
        library.name = remap.map_string(library.name)?;
        library.version = library.version.map(|id| remap.map_string(id)).transpose()?;
        out.ffi_libraries.push(library);
    }
    for layout in &source.ffi_layouts {
        let mut layout = layout.clone();
        layout.name = remap.map_string(layout.name)?;
        layout.verum_type = layout
            .verum_type
            .map(|id| remap.map_type_id(id))
            .transpose()?;
        for field in &mut layout.fields {
            // The existing VBC field key belongs to the codegen field interner.
            // Native marshalling uses declaration order and packed byte offsets.
            field.nested_layout = field
                .nested_layout
                .map(|id| remap.ffi.layout(id))
                .transpose()?;
        }
        out.ffi_layouts.push(layout);
    }
    for symbol in &source.ffi_symbols {
        let mut symbol = symbol.clone();
        symbol.name = remap.map_string(symbol.name)?;
        if symbol.library_idx != -1 {
            symbol.library_idx = remap
                .ffi
                .libraries
                .map(symbol.library_idx as u32, "FFI library")?
                as i16;
        }
        symbol.wrapper_fn = symbol
            .wrapper_fn
            .map(|id| remap.map_function(id))
            .transpose()?;
        let signature = &mut symbol.signature;
        signature.return_layout_idx = signature
            .return_layout_idx
            .map(|id| remap.ffi.layout(id))
            .transpose()?;
        for index in &mut signature.param_layout_indices {
            *index = index.map(|id| remap.ffi.layout(id)).transpose()?;
        }
        out.ffi_symbols.push(symbol);
    }
    Ok(())
}
