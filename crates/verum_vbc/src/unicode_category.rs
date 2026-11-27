//! Unicode 17.0 General_Category codes used by the scalar VBC intrinsic.
//! Generated from DerivedGeneralCategory.txt by scripts/generate_unicode_categories.py.
pub const PAGE_COUNT: usize = 0x1100;
/// First the one-byte page indices, then the distinct 256-byte category pages.
pub const TABLE: &[u8] = include_bytes!("unicode_category/ucd17.bin");

/// Return the intrinsic's category code for a valid Verum Char.
pub fn code(scalar: char) -> u8 {
    let point = scalar as usize;
    let page = usize::from(TABLE[point >> 8]);
    TABLE[PAGE_COUNT + page * 256 + (point & 255)]
}

/// The call-site and callable forms execute the same scalar operation.
pub fn instruction(dst: crate::Reg, src: crate::Reg) -> crate::Instruction {
    let mut operands = Default::default();
    crate::encoding::encode_reg(dst, &mut operands);
    crate::encoding::encode_reg(src, &mut operands);
    crate::Instruction::CharExtended {
        sub_op: crate::instruction::CharSubOpcode::GeneralCategory.to_byte(),
        operands,
    }
}
