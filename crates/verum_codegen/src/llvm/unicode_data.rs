//! Unicode range tables and LLVM emission helpers.
//!
//! This module replaces the C runtime Unicode classification and case conversion
//! functions with LLVM global constants + inline binary search / linear scan.
//! The tables are emitted once per module and shared across all call sites.

use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::BasicType;
use verum_llvm::values::{FunctionValue, GlobalValue, IntValue, PointerValue};
use verum_llvm::{AddressSpace, IntPredicate};

use super::context::FunctionContext;
use super::error::{BuildExt, LlvmLoweringError, OptionExt, Result};

// ---------------------------------------------------------------------------
// Table data — Unicode character classification tables
// ---------------------------------------------------------------------------

/// Unicode Alphabetic ranges (239 pairs).
pub const ALPHA_RANGES: &[(u32, u32)] = &[
    (0x0041, 0x005A), (0x0061, 0x007A), (0x00AA, 0x00AA), (0x00B5, 0x00B5),
    (0x00BA, 0x00BA), (0x00C0, 0x00D6), (0x00D8, 0x00F6), (0x00F8, 0x02C1),
    (0x02C6, 0x02D1), (0x02E0, 0x02E4), (0x0370, 0x0374), (0x0376, 0x0377),
    (0x037A, 0x037D), (0x037F, 0x037F), (0x0386, 0x0386), (0x0388, 0x038A),
    (0x038C, 0x038C), (0x038E, 0x03A1), (0x03A3, 0x03F5), (0x03F7, 0x0481),
    (0x048A, 0x052F), (0x0531, 0x0556), (0x0560, 0x0588), (0x05D0, 0x05EA),
    (0x05EF, 0x05F2), (0x0620, 0x064A), (0x066E, 0x066F), (0x0671, 0x06D3),
    (0x06D5, 0x06D5), (0x06E5, 0x06E6), (0x06EE, 0x06EF), (0x06FA, 0x06FC),
    (0x06FF, 0x06FF), (0x0710, 0x0710), (0x0712, 0x072F), (0x074D, 0x07A5),
    (0x07B1, 0x07B1), (0x07CA, 0x07EA), (0x0800, 0x0815), (0x0904, 0x0939),
    (0x093D, 0x093D), (0x0950, 0x0950), (0x0958, 0x0961), (0x0971, 0x0980),
    (0x0985, 0x098C), (0x098F, 0x0990), (0x0993, 0x09A8), (0x09AA, 0x09B0),
    (0x09B2, 0x09B2), (0x09B6, 0x09B9), (0x09BD, 0x09BD), (0x09CE, 0x09CE),
    (0x09DC, 0x09DD), (0x09DF, 0x09E1), (0x09F0, 0x09F1), (0x0A05, 0x0A0A),
    (0x0A0F, 0x0A10), (0x0A13, 0x0A28), (0x0A2A, 0x0A30), (0x0A32, 0x0A33),
    (0x0A35, 0x0A36), (0x0A38, 0x0A39), (0x0A59, 0x0A5C), (0x0A5E, 0x0A5E),
    (0x0A72, 0x0A74), (0x1000, 0x102A), (0x1050, 0x1055), (0x10A0, 0x10C5),
    (0x10C7, 0x10C7), (0x10CD, 0x10CD), (0x10D0, 0x10FA), (0x10FC, 0x1248),
    (0x1250, 0x1256), (0x1258, 0x1258), (0x125A, 0x125D), (0x1260, 0x1288),
    (0x128A, 0x128D), (0x1290, 0x12B0), (0x12B2, 0x12B5), (0x12B8, 0x12BE),
    (0x12C0, 0x12C0), (0x12C2, 0x12C5), (0x12C8, 0x12D6), (0x12D8, 0x1310),
    (0x1312, 0x1315), (0x1318, 0x135A), (0x1380, 0x138F), (0x13A0, 0x13F5),
    (0x13F8, 0x13FD), (0x1401, 0x166C), (0x166F, 0x167F), (0x1681, 0x169A),
    (0x16A0, 0x16EA), (0x1700, 0x170C), (0x170E, 0x1711), (0x1780, 0x17B3),
    (0x1820, 0x1878), (0x1900, 0x191E), (0x1950, 0x196D), (0x1970, 0x1974),
    (0x1980, 0x19AB), (0x19B0, 0x19C9), (0x1A00, 0x1A16), (0x1B05, 0x1B33),
    (0x1B45, 0x1B4B), (0x2C00, 0x2C2E), (0x2C30, 0x2C5E), (0x2C60, 0x2CE4),
    (0x2CEB, 0x2CEE), (0x2D00, 0x2D25), (0x2D27, 0x2D27), (0x2D2D, 0x2D2D),
    (0x2D30, 0x2D67), (0x2D6F, 0x2D6F), (0x2D80, 0x2D96), (0x2DA0, 0x2DA6),
    (0x2DA8, 0x2DAE), (0x2DB0, 0x2DB6), (0x2DB8, 0x2DBE), (0x2DC0, 0x2DC6),
    (0x2DC8, 0x2DCE), (0x2DD0, 0x2DD6), (0x2DD8, 0x2DDE), (0x3005, 0x3005),
    (0x3006, 0x3006), (0x3007, 0x3007), (0x3021, 0x3029), (0x3031, 0x3035),
    (0x3038, 0x303C), (0x3041, 0x3096), (0x309D, 0x309F), (0x30A1, 0x30FA),
    (0x30FC, 0x30FF), (0x3105, 0x312F), (0x3131, 0x318E), (0x31A0, 0x31BF),
    (0x31F0, 0x31FF), (0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xA000, 0xA48C),
    (0xA4D0, 0xA4FD), (0xA500, 0xA60C), (0xA610, 0xA61F), (0xA62A, 0xA62B),
    (0xA640, 0xA66E), (0xA67F, 0xA69D), (0xA6A0, 0xA6E5), (0xA717, 0xA71F),
    (0xA722, 0xA788), (0xA78B, 0xA7CA), (0xA7D0, 0xA7D1), (0xA7D3, 0xA7D3),
    (0xA7D5, 0xA7D9), (0xA7F2, 0xA801), (0xA803, 0xA805), (0xA807, 0xA80A),
    (0xA80C, 0xA822), (0xAC00, 0xD7A3), (0xD7B0, 0xD7C6), (0xD7CB, 0xD7FB),
    (0xF900, 0xFA6D), (0xFA70, 0xFAD9), (0xFB00, 0xFB06), (0xFB13, 0xFB17),
    (0xFB1D, 0xFB1D), (0xFB1F, 0xFB28), (0xFB2A, 0xFB36), (0xFB38, 0xFB3C),
    (0xFB3E, 0xFB3E), (0xFB40, 0xFB41), (0xFB43, 0xFB44), (0xFB46, 0xFBB1),
    (0xFBD3, 0xFD3D), (0xFD50, 0xFD8F), (0xFD92, 0xFDC7), (0xFDF0, 0xFDFB),
    (0xFE70, 0xFE74), (0xFE76, 0xFEFC), (0xFF21, 0xFF3A), (0xFF41, 0xFF5A),
    (0xFF66, 0xFFBE), (0xFFC2, 0xFFC7), (0xFFCA, 0xFFCF), (0xFFD2, 0xFFD7),
    (0xFFDA, 0xFFDC), (0x10000, 0x1000B), (0x1000D, 0x10026), (0x10028, 0x1003A),
    (0x1003C, 0x1003D), (0x1003F, 0x1004D), (0x10050, 0x1005D), (0x10080, 0x100FA),
    (0x10280, 0x1029C), (0x102A0, 0x102D0), (0x10300, 0x1031F), (0x1032D, 0x10340),
    (0x10342, 0x10349), (0x10400, 0x1044F), (0x10450, 0x1049D), (0x10500, 0x10527),
    (0x10530, 0x10563), (0x10600, 0x10736), (0x10800, 0x10805), (0x1D400, 0x1D454),
    (0x1D456, 0x1D49C), (0x1D49E, 0x1D49F), (0x1D4A2, 0x1D4A2), (0x1D4A5, 0x1D4A6),
    (0x1D4A9, 0x1D4AC), (0x1D4AE, 0x1D4B9), (0x1D4BB, 0x1D4BB), (0x1D4BD, 0x1D4C3),
    (0x1D4C5, 0x1D505), (0x1D507, 0x1D50A), (0x1D50D, 0x1D514), (0x1D516, 0x1D51C),
    (0x1D51E, 0x1D539), (0x1D53B, 0x1D53E), (0x1D540, 0x1D544), (0x1D546, 0x1D546),
    (0x1D54A, 0x1D550), (0x1D552, 0x1D6A5), (0x1D6A8, 0x1D6C0), (0x1D6C2, 0x1D6DA),
    (0x1D6DC, 0x1D6FA), (0x1D6FC, 0x1D714), (0x1D716, 0x1D734), (0x1D736, 0x1D74E),
    (0x1D750, 0x1D76E), (0x1D770, 0x1D788), (0x1D78A, 0x1D7A8), (0x1D7AA, 0x1D7C2),
    (0x1D7C4, 0x1D7CB), (0x20000, 0x2A6DF), (0x2A700, 0x2B739), (0x2B740, 0x2B81D),
    (0x2B820, 0x2CEA1), (0x2CEB0, 0x2EBE0), (0x30000, 0x3134A),
];

/// Unicode Numeric ranges (58 pairs).
pub const NUMERIC_RANGES: &[(u32, u32)] = &[
    (0x0030, 0x0039), (0x0660, 0x0669), (0x06F0, 0x06F9), (0x07C0, 0x07C9),
    (0x0966, 0x096F), (0x09E6, 0x09EF), (0x0A66, 0x0A6F), (0x0AE6, 0x0AEF),
    (0x0B66, 0x0B6F), (0x0BE6, 0x0BEF), (0x0C66, 0x0C6F), (0x0CE6, 0x0CEF),
    (0x0D66, 0x0D6F), (0x0DE6, 0x0DEF), (0x0E50, 0x0E59), (0x0ED0, 0x0ED9),
    (0x0F20, 0x0F29), (0x1040, 0x1049), (0x1090, 0x1099), (0x17E0, 0x17E9),
    (0x1810, 0x1819), (0x1946, 0x194F), (0x19D0, 0x19D9), (0x1A80, 0x1A89),
    (0x1A90, 0x1A99), (0x1B50, 0x1B59), (0x1BB0, 0x1BB9), (0x1C40, 0x1C49),
    (0x1C50, 0x1C59), (0xA620, 0xA629), (0xA8D0, 0xA8D9), (0xA900, 0xA909),
    (0xA9D0, 0xA9D9), (0xA9F0, 0xA9F9), (0xAA50, 0xAA59), (0xABF0, 0xABF9),
    (0xFF10, 0xFF19), (0x104A0, 0x104A9), (0x11066, 0x1106F), (0x110F0, 0x110F9),
    (0x11136, 0x1113F), (0x111D0, 0x111D9), (0x112F0, 0x112F9), (0x11450, 0x11459),
    (0x114D0, 0x114D9), (0x11650, 0x11659), (0x116C0, 0x116C9), (0x11730, 0x11739),
    (0x118E0, 0x118E9), (0x11950, 0x11959), (0x11C50, 0x11C59), (0x11D50, 0x11D59),
    (0x11DA0, 0x11DA9), (0x1D7CE, 0x1D7FF), (0x1E140, 0x1E149), (0x1E2F0, 0x1E2F9),
    (0x1E950, 0x1E959), (0x1FBF0, 0x1FBF9),
];

/// Unicode Whitespace ranges (10 pairs).
pub const WS_RANGES: &[(u32, u32)] = &[
    (0x0009, 0x000D), (0x0020, 0x0020), (0x0085, 0x0085), (0x00A0, 0x00A0),
    (0x1680, 0x1680), (0x2000, 0x200A), (0x2028, 0x2029), (0x202F, 0x202F),
    (0x205F, 0x205F), (0x3000, 0x3000),
];

/// Unicode Lowercase ranges (101 pairs).
pub const LOWER_RANGES: &[(u32, u32)] = &[
    (0x0061, 0x007A), (0x00B5, 0x00B5), (0x00DF, 0x00F6), (0x00F8, 0x00FF),
    (0x0101, 0x0101), (0x0137, 0x0138), (0x017A, 0x017A), (0x017C, 0x017C),
    (0x017E, 0x0180), (0x0199, 0x019B), (0x01A1, 0x01A1), (0x01B4, 0x01B4),
    (0x01CE, 0x01CE), (0x01DD, 0x01DD), (0x0250, 0x02AF), (0x0371, 0x0371),
    (0x0373, 0x0373), (0x0377, 0x0377), (0x037B, 0x037D), (0x0390, 0x0390),
    (0x03AC, 0x03CE), (0x03D0, 0x03D1), (0x03D5, 0x03D7), (0x03D9, 0x03D9),
    (0x03EF, 0x03EF), (0x03F0, 0x03F3), (0x03F5, 0x03F5), (0x03F8, 0x03F8),
    (0x03FB, 0x03FC), (0x0430, 0x045F), (0x0461, 0x0461), (0x048B, 0x048B),
    (0x04CE, 0x04CF), (0x0560, 0x0588), (0x10D0, 0x10FA), (0x10FD, 0x10FF),
    (0x13F8, 0x13FD), (0x1C80, 0x1C88), (0x1D00, 0x1DBF), (0x1E01, 0x1E01),
    (0x1E95, 0x1E9D), (0x1E9F, 0x1E9F), (0x1EA1, 0x1EA1), (0x1EFF, 0x1EFF),
    (0x1F00, 0x1F07), (0x1F10, 0x1F15), (0x1F20, 0x1F27), (0x1F30, 0x1F37),
    (0x1F40, 0x1F45), (0x1F50, 0x1F57), (0x1F60, 0x1F67), (0x1F70, 0x1F7D),
    (0x1F80, 0x1F87), (0x1F90, 0x1F97), (0x1FA0, 0x1FA7), (0x1FB0, 0x1FB4),
    (0x1FB6, 0x1FB7), (0x1FBE, 0x1FBE), (0x1FC2, 0x1FC4), (0x1FC6, 0x1FC7),
    (0x1FD0, 0x1FD3), (0x1FD6, 0x1FD7), (0x1FE0, 0x1FE7), (0x1FF2, 0x1FF4),
    (0x1FF6, 0x1FF7), (0x2071, 0x2071), (0x207F, 0x207F), (0x2090, 0x209C),
    (0x210A, 0x210A), (0x210E, 0x210F), (0x2113, 0x2113), (0x212F, 0x212F),
    (0x2134, 0x2134), (0x2139, 0x2139), (0x213C, 0x213D), (0x2146, 0x2149),
    (0x214E, 0x214E), (0x2170, 0x217F), (0x2184, 0x2184), (0x24D0, 0x24E9),
    (0x2C30, 0x2C5E), (0xFF41, 0xFF5A), (0x10428, 0x1044F), (0x104D8, 0x104FB),
    (0x10CC0, 0x10CF2), (0x118C0, 0x118DF), (0x16E60, 0x16E7F),
    (0x1D41A, 0x1D433), (0x1D44E, 0x1D467), (0x1D482, 0x1D49B),
    (0x1D4B6, 0x1D4CF), (0x1D4EA, 0x1D503), (0x1D51E, 0x1D537),
    (0x1D552, 0x1D56B), (0x1D586, 0x1D59F), (0x1D5BA, 0x1D5D3),
    (0x1D5EE, 0x1D607), (0x1D622, 0x1D63B), (0x1D656, 0x1D66F),
    (0x1D68A, 0x1D6A5), (0x1E922, 0x1E943),
];

/// Unicode Uppercase ranges (129 pairs).
pub const UPPER_RANGES: &[(u32, u32)] = &[
    (0x0041, 0x005A), (0x00C0, 0x00D6), (0x00D8, 0x00DE), (0x0100, 0x0100),
    (0x0136, 0x0136), (0x0139, 0x0139), (0x0178, 0x0179), (0x017B, 0x017B),
    (0x017D, 0x017D), (0x0181, 0x0182), (0x0186, 0x0187), (0x0189, 0x018B),
    (0x018E, 0x0191), (0x0193, 0x0194), (0x0196, 0x0198), (0x019C, 0x019D),
    (0x019F, 0x01A0), (0x01A2, 0x01A2), (0x01A4, 0x01A4), (0x01A7, 0x01A7),
    (0x01AC, 0x01AC), (0x01AF, 0x01AF), (0x01B1, 0x01B3), (0x01B5, 0x01B5),
    (0x01B7, 0x01B8), (0x01BC, 0x01BC), (0x01C4, 0x01C4), (0x01C7, 0x01C7),
    (0x01CA, 0x01CA), (0x01CD, 0x01CD), (0x01CF, 0x01CF), (0x01F1, 0x01F1),
    (0x01F4, 0x01F4), (0x01F6, 0x01F8), (0x0200, 0x0200), (0x0241, 0x0241),
    (0x0243, 0x0246), (0x0248, 0x0248), (0x024A, 0x024A), (0x024C, 0x024C),
    (0x024E, 0x024E), (0x0386, 0x0386), (0x0388, 0x038A), (0x038C, 0x038C),
    (0x038E, 0x038F), (0x0391, 0x03A1), (0x03A3, 0x03AB), (0x03D2, 0x03D4),
    (0x03D8, 0x03D8), (0x03F4, 0x03F4), (0x03F7, 0x03F7), (0x03F9, 0x03FA),
    (0x03FD, 0x042F), (0x0460, 0x0460), (0x048A, 0x048A), (0x04C0, 0x04C1),
    (0x0531, 0x0556), (0x10A0, 0x10C5), (0x10C7, 0x10C7), (0x10CD, 0x10CD),
    (0x13A0, 0x13F5), (0x1C90, 0x1CBA), (0x1CBD, 0x1CBF), (0x1E00, 0x1E00),
    (0x1E94, 0x1E94), (0x1E9E, 0x1E9E), (0x1EA0, 0x1EA0), (0x1EFE, 0x1EFE),
    (0x1F08, 0x1F0F), (0x1F18, 0x1F1D), (0x1F28, 0x1F2F), (0x1F38, 0x1F3F),
    (0x1F48, 0x1F4D), (0x1F59, 0x1F5F), (0x1F68, 0x1F6F), (0x1FB8, 0x1FBB),
    (0x1FC8, 0x1FCB), (0x1FD8, 0x1FDB), (0x1FE8, 0x1FEC), (0x1FF8, 0x1FFB),
    (0x2102, 0x2102), (0x2107, 0x2107), (0x210B, 0x210D), (0x2110, 0x2112),
    (0x2115, 0x2115), (0x2119, 0x211D), (0x2124, 0x2124), (0x2126, 0x2126),
    (0x2128, 0x2128), (0x212A, 0x212D), (0x2130, 0x2133), (0x213E, 0x213F),
    (0x2145, 0x2145), (0x2160, 0x216F), (0x2183, 0x2183), (0x24B6, 0x24CF),
    (0x2C00, 0x2C2E), (0xFF21, 0xFF3A), (0x10400, 0x10427), (0x104B0, 0x104D3),
    (0x10C80, 0x10CB2), (0x118A0, 0x118BF), (0x16E40, 0x16E5F),
    (0x1D400, 0x1D419), (0x1D434, 0x1D44D), (0x1D468, 0x1D481),
    (0x1D49C, 0x1D49C), (0x1D49E, 0x1D49F), (0x1D4A2, 0x1D4A2),
    (0x1D4A5, 0x1D4A6), (0x1D4A9, 0x1D4AC), (0x1D4AE, 0x1D4B5),
    (0x1D4D0, 0x1D4E9), (0x1D504, 0x1D505), (0x1D507, 0x1D50A),
    (0x1D50D, 0x1D514), (0x1D516, 0x1D51C), (0x1D538, 0x1D539),
    (0x1D53B, 0x1D53E), (0x1D540, 0x1D544), (0x1D546, 0x1D546),
    (0x1D54A, 0x1D550), (0x1D56C, 0x1D585), (0x1D5A0, 0x1D5B9),
    (0x1D5D4, 0x1D5ED), (0x1D608, 0x1D621), (0x1D63C, 0x1D655),
    (0x1D670, 0x1D689), (0x1E900, 0x1E921),
];

/// Case conversion pairs (87 pairs): (lowercase, uppercase).
pub const CASE_PAIRS: &[(u32, u32)] = &[
    // Latin Extended-A
    (0x00E0, 0x00C0), (0x00E1, 0x00C1), (0x00E2, 0x00C2), (0x00E3, 0x00C3),
    (0x00E4, 0x00C4), (0x00E5, 0x00C5), (0x00E6, 0x00C6), (0x00E7, 0x00C7),
    (0x00E8, 0x00C8), (0x00E9, 0x00C9), (0x00EA, 0x00CA), (0x00EB, 0x00CB),
    (0x00EC, 0x00CC), (0x00ED, 0x00CD), (0x00EE, 0x00CE), (0x00EF, 0x00CF),
    (0x00F0, 0x00D0), (0x00F1, 0x00D1), (0x00F2, 0x00D2), (0x00F3, 0x00D3),
    (0x00F4, 0x00D4), (0x00F5, 0x00D5), (0x00F6, 0x00D6), (0x00F8, 0x00D8),
    (0x00F9, 0x00D9), (0x00FA, 0x00DA), (0x00FB, 0x00DB), (0x00FC, 0x00DC),
    (0x00FD, 0x00DD), (0x00FE, 0x00DE), (0x00FF, 0x0178),
    // Greek
    (0x03B1, 0x0391), (0x03B2, 0x0392), (0x03B3, 0x0393), (0x03B4, 0x0394),
    (0x03B5, 0x0395), (0x03B6, 0x0396), (0x03B7, 0x0397), (0x03B8, 0x0398),
    (0x03B9, 0x0399), (0x03BA, 0x039A), (0x03BB, 0x039B), (0x03BC, 0x039C),
    (0x03BD, 0x039D), (0x03BE, 0x039E), (0x03BF, 0x039F), (0x03C0, 0x03A0),
    (0x03C1, 0x03A1), (0x03C3, 0x03A3), (0x03C4, 0x03A4), (0x03C5, 0x03A5),
    (0x03C6, 0x03A6), (0x03C7, 0x03A7), (0x03C8, 0x03A8), (0x03C9, 0x03A9),
    // Cyrillic
    (0x0430, 0x0410), (0x0431, 0x0411), (0x0432, 0x0412), (0x0433, 0x0413),
    (0x0434, 0x0414), (0x0435, 0x0415), (0x0436, 0x0416), (0x0437, 0x0417),
    (0x0438, 0x0418), (0x0439, 0x0419), (0x043A, 0x041A), (0x043B, 0x041B),
    (0x043C, 0x041C), (0x043D, 0x041D), (0x043E, 0x041E), (0x043F, 0x041F),
    (0x0440, 0x0420), (0x0441, 0x0421), (0x0442, 0x0422), (0x0443, 0x0423),
    (0x0444, 0x0424), (0x0445, 0x0425), (0x0446, 0x0426), (0x0447, 0x0427),
    (0x0448, 0x0428), (0x0449, 0x0429), (0x044A, 0x042A), (0x044B, 0x042B),
    (0x044C, 0x042C), (0x044D, 0x042D), (0x044E, 0x042E), (0x044F, 0x042F),
];

// ---------------------------------------------------------------------------
// LLVM emission helpers
// ---------------------------------------------------------------------------

/// Get or create a global i32 array for a Unicode range table.
/// The table is flattened: [lo0, hi0, lo1, hi1, ...].
/// Returns the global and the pair count.
pub fn get_or_create_range_table<'ctx>(
    module: &Module<'ctx>,
    context: &'ctx Context,
    name: &str,
    data: &[(u32, u32)],
) -> (GlobalValue<'ctx>, u32) {
    if let Some(gv) = module.get_global(name) {
        return (gv, data.len() as u32);
    }
    let i32_ty = context.i32_type();
    let flat_len = (data.len() * 2) as u32;
    let arr_ty = i32_ty.array_type(flat_len);
    let values: Vec<_> = data
        .iter()
        .flat_map(|(lo, hi)| {
            [
                i32_ty.const_int(*lo as u64, false),
                i32_ty.const_int(*hi as u64, false),
            ]
        })
        .collect();
    let arr_val = i32_ty.const_array(&values);
    let gv = module.add_global(arr_ty, None, name);
    gv.set_initializer(&arr_val);
    gv.set_constant(true);
    gv.set_unnamed_addr(true);
    (gv, data.len() as u32)
}

/// Emit a Unicode range table lookup using the shared binary search function.
/// Returns an i64 value that is 0 or 1.
pub fn emit_range_table_lookup<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    ch: IntValue<'ctx>,
    table_name: &str,
    table_data: &[(u32, u32)],
) -> Result<IntValue<'ctx>> {
    let i32_ty = ctx.llvm_context().i32_type();
    let module = ctx.get_module();

    let (global, pair_count) =
        get_or_create_range_table(module, ctx.llvm_context(), table_name, table_data);

    let search_fn = get_or_create_search_fn(module, ctx.llvm_context())?;

    let table_ptr = global.as_pointer_value();
    let count_val = i32_ty.const_int(pair_count as u64, false);

    let call_result = ctx
        .builder()
        .build_call(
            search_fn,
            &[ch.into(), table_ptr.into(), count_val.into()],
            "unicode_lookup",
        )
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| LlvmLoweringError::internal("unicode range lookup: expected return value"))?
        .into_int_value();

    Ok(call_result)
}

/// Get or create `__unicode_in_range_table(i64 cp, ptr table, i32 count) -> i64`.
/// Binary search over sorted [lo, hi] pairs. Returns 0 or 1.
fn get_or_create_search_fn<'ctx>(
    module: &Module<'ctx>,
    context: &'ctx Context,
) -> Result<FunctionValue<'ctx>> {
    let fn_name = "__unicode_in_range_table";
    if let Some(f) = module.get_function(fn_name) {
        return Ok(f);
    }

    let i64_ty = context.i64_type();
    let i32_ty = context.i32_type();
    let ptr_ty = context.ptr_type(AddressSpace::default());
    let fn_type = i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into(), i32_ty.into()], false);
    let func = module.add_function(fn_name, fn_type, None);
    func.set_linkage(verum_llvm::module::Linkage::Internal);

    let entry_bb = context.append_basic_block(func, "entry");
    let loop_bb = context.append_basic_block(func, "loop");
    let check_bb = context.append_basic_block(func, "check");
    let reduce_hi_bb = context.append_basic_block(func, "reduce_hi");
    let check_hi_bb = context.append_basic_block(func, "check_hi");
    let advance_lo_bb = context.append_basic_block(func, "advance_lo");
    let found_bb = context.append_basic_block(func, "found");
    let not_found_bb = context.append_basic_block(func, "not_found");

    let builder = context.create_builder();

    // entry: truncate cp to i32, compute hi_init = count - 1
    builder.position_at_end(entry_bb);
    let cp = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
    let table_ptr = func.get_nth_param(1).or_internal("missing param 1")?.into_pointer_value();
    let count = func.get_nth_param(2).or_internal("missing param 2")?.into_int_value();
    let cp_i32 = builder
        .build_int_truncate(cp, i32_ty, "cp32")
        .or_llvm_err()?;
    let hi_init = builder
        .build_int_sub(count, i32_ty.const_int(1, false), "hi_init")
        .or_llvm_err()?;
    builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

    // loop: phi lo, phi hi; branch to check if lo <= hi, else not_found
    builder.position_at_end(loop_bb);
    let lo_phi = builder.build_phi(i32_ty, "lo").or_llvm_err()?;
    let hi_phi = builder.build_phi(i32_ty, "hi").or_llvm_err()?;
    let lo_val = lo_phi.as_basic_value().into_int_value();
    let hi_val = hi_phi.as_basic_value().into_int_value();
    let cond = builder
        .build_int_compare(IntPredicate::SLE, lo_val, hi_val, "lo_le_hi")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(cond, check_bb, not_found_bb)
        .or_llvm_err()?;

    // check: compute mid, load table[mid*2] and table[mid*2+1]
    builder.position_at_end(check_bb);
    let sum = builder.build_int_add(lo_val, hi_val, "sum").or_llvm_err()?;
    let mid = builder
        .build_int_signed_div(sum, i32_ty.const_int(2, false), "mid")
        .or_llvm_err()?;
    let mid_x2 = builder
        .build_int_mul(mid, i32_ty.const_int(2, false), "mid_x2")
        .or_llvm_err()?;
    let mid_x2_p1 = builder
        .build_int_add(mid_x2, i32_ty.const_int(1, false), "mid_x2_p1")
        .or_llvm_err()?;
    let range_lo_ptr = unsafe {
        builder
            .build_in_bounds_gep(i32_ty, table_ptr, &[mid_x2], "rlo_ptr")
            .or_llvm_err()?
    };
    let range_lo = builder
        .build_load(i32_ty, range_lo_ptr, "range_lo")
        .or_llvm_err()?
        .into_int_value();
    let range_hi_ptr = unsafe {
        builder
            .build_in_bounds_gep(i32_ty, table_ptr, &[mid_x2_p1], "rhi_ptr")
            .or_llvm_err()?
    };
    let range_hi = builder
        .build_load(i32_ty, range_hi_ptr, "range_hi")
        .or_llvm_err()?
        .into_int_value();

    // if cp < range_lo → reduce_hi, else → check_hi
    let lt_lo = builder
        .build_int_compare(IntPredicate::ULT, cp_i32, range_lo, "lt_lo")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(lt_lo, reduce_hi_bb, check_hi_bb)
        .or_llvm_err()?;

    // reduce_hi: hi = mid - 1, loop back
    builder.position_at_end(reduce_hi_bb);
    let new_hi = builder
        .build_int_sub(mid, i32_ty.const_int(1, false), "new_hi")
        .or_llvm_err()?;
    builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

    // check_hi: if cp > range_hi → advance_lo, else → found
    builder.position_at_end(check_hi_bb);
    let gt_hi = builder
        .build_int_compare(IntPredicate::UGT, cp_i32, range_hi, "gt_hi")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(gt_hi, advance_lo_bb, found_bb)
        .or_llvm_err()?;

    // advance_lo: lo = mid + 1, loop back
    builder.position_at_end(advance_lo_bb);
    let new_lo = builder
        .build_int_add(mid, i32_ty.const_int(1, false), "new_lo")
        .or_llvm_err()?;
    builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

    // found: return 1
    builder.position_at_end(found_bb);
    builder
        .build_return(Some(&i64_ty.const_int(1, false)))
        .or_llvm_err()?;

    // not_found: return 0
    builder.position_at_end(not_found_bb);
    builder.build_return(Some(&i64_ty.const_zero())).or_llvm_err()?;

    // Wire phi incoming edges
    lo_phi.add_incoming(&[
        (&i32_ty.const_zero(), entry_bb),
        (&lo_val, reduce_hi_bb),
        (&new_lo, advance_lo_bb),
    ]);
    hi_phi.add_incoming(&[
        (&hi_init, entry_bb),
        (&new_hi, reduce_hi_bb),
        (&hi_val, advance_lo_bb),
    ]);

    Ok(func)
}

/// Emit Unicode case conversion.
/// `direction`: 0 = to_uppercase, 1 = to_lowercase.
/// Returns the converted codepoint as i64.
pub fn emit_case_conversion<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    ch: IntValue<'ctx>,
    direction: u8,
) -> Result<IntValue<'ctx>> {
    let i32_ty = ctx.llvm_context().i32_type();
    let module = ctx.get_module();
    let context = ctx.llvm_context();

    let convert_fn = get_or_create_case_convert_fn(module, context)?;

    let dir_val = i32_ty.const_int(direction as u64, false);
    let call_result = ctx
        .builder()
        .build_call(convert_fn, &[ch.into(), dir_val.into()], "case_conv")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| LlvmLoweringError::internal("case conversion: expected return value"))?
        .into_int_value();

    Ok(call_result)
}

/// Get or create `__unicode_case_convert(i64 cp, i32 direction) -> i64`.
/// direction=0: to_uppercase (search lowercase col, return uppercase),
/// direction=1: to_lowercase (search uppercase col, return lowercase).
fn get_or_create_case_convert_fn<'ctx>(
    module: &Module<'ctx>,
    context: &'ctx Context,
) -> Result<FunctionValue<'ctx>> {
    let fn_name = "__unicode_case_convert";
    if let Some(f) = module.get_function(fn_name) {
        return Ok(f);
    }

    let i64_ty = context.i64_type();
    let i32_ty = context.i32_type();
    let fn_type = i64_ty.fn_type(&[i64_ty.into(), i32_ty.into()], false);
    let func = module.add_function(fn_name, fn_type, None);
    func.set_linkage(verum_llvm::module::Linkage::Internal);

    let entry_bb = context.append_basic_block(func, "entry");
    let ascii_upper_bb = context.append_basic_block(func, "ascii_upper");
    let ascii_lower_bb = context.append_basic_block(func, "ascii_lower");
    let table_scan_bb = context.append_basic_block(func, "table_scan");
    let loop_bb = context.append_basic_block(func, "loop");
    let loop_body_bb = context.append_basic_block(func, "loop_body");
    let found_bb = context.append_basic_block(func, "found");
    let inc_bb = context.append_basic_block(func, "inc");
    let not_found_bb = context.append_basic_block(func, "not_found");
    let upper_ascii_ret_bb = context.append_basic_block(func, "upper_ascii_ret");
    let lower_ascii_ret_bb = context.append_basic_block(func, "lower_ascii_ret");

    let builder = context.create_builder();

    let cp = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
    let direction = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

    // Create the case pairs global constant
    let table_name = "__unicode_case_pairs";
    let pair_count = CASE_PAIRS.len() as u32;
    if module.get_global(table_name).is_none() {
        let flat_len = (CASE_PAIRS.len() * 2) as u32;
        let arr_ty = i32_ty.array_type(flat_len);
        let values: Vec<_> = CASE_PAIRS
            .iter()
            .flat_map(|(lo, hi)| {
                [
                    i32_ty.const_int(*lo as u64, false),
                    i32_ty.const_int(*hi as u64, false),
                ]
            })
            .collect();
        let arr_val = i32_ty.const_array(&values);
        let gv = module.add_global(arr_ty, None, table_name);
        gv.set_initializer(&arr_val);
        gv.set_constant(true);
        gv.set_unnamed_addr(true);
    }
    let pairs_ptr = module.get_global(table_name).or_internal("case_pairs global")?.as_pointer_value();

    // entry: branch on direction
    builder.position_at_end(entry_bb);
    let is_to_upper = builder
        .build_int_compare(IntPredicate::EQ, direction, i32_ty.const_zero(), "is_to_upper")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(is_to_upper, ascii_upper_bb, ascii_lower_bb)
        .or_llvm_err()?;

    // ascii_upper: if cp in [0x61..0x7A] → cp - 32
    builder.position_at_end(ascii_upper_bb);
    let ge_a = builder
        .build_int_compare(IntPredicate::SGE, cp, i64_ty.const_int(0x61, false), "ge_a")
        .or_llvm_err()?;
    let le_z = builder
        .build_int_compare(IntPredicate::SLE, cp, i64_ty.const_int(0x7A, false), "le_z")
        .or_llvm_err()?;
    let is_ascii_lower = builder.build_and(ge_a, le_z, "is_ascii_lower").or_llvm_err()?;
    builder
        .build_conditional_branch(is_ascii_lower, upper_ascii_ret_bb, table_scan_bb)
        .or_llvm_err()?;

    builder.position_at_end(upper_ascii_ret_bb);
    let upper_val = builder
        .build_int_sub(cp, i64_ty.const_int(32, false), "upper_val")
        .or_llvm_err()?;
    builder.build_return(Some(&upper_val)).or_llvm_err()?;

    // ascii_lower: if cp in [0x41..0x5A] → cp + 32
    builder.position_at_end(ascii_lower_bb);
    let ge_upper_a = builder
        .build_int_compare(IntPredicate::SGE, cp, i64_ty.const_int(0x41, false), "ge_A")
        .or_llvm_err()?;
    let le_upper_z = builder
        .build_int_compare(IntPredicate::SLE, cp, i64_ty.const_int(0x5A, false), "le_Z")
        .or_llvm_err()?;
    let is_ascii_upper = builder
        .build_and(ge_upper_a, le_upper_z, "is_ascii_upper")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(is_ascii_upper, lower_ascii_ret_bb, table_scan_bb)
        .or_llvm_err()?;

    builder.position_at_end(lower_ascii_ret_bb);
    let lower_val = builder
        .build_int_add(cp, i64_ty.const_int(32, false), "lower_val")
        .or_llvm_err()?;
    builder.build_return(Some(&lower_val)).or_llvm_err()?;

    // table_scan: linear scan over CASE_PAIRS
    builder.position_at_end(table_scan_bb);
    let cp_i32 = builder.build_int_truncate(cp, i32_ty, "cp32").or_llvm_err()?;
    // search_offset = direction (0 for to_upper searches lowercase col, 1 for to_lower searches uppercase col)
    let result_offset = builder
        .build_int_sub(i32_ty.const_int(1, false), direction, "result_off")
        .or_llvm_err()?;
    builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

    // loop: i = phi(0, i+1); if i < count → body, else → not_found
    builder.position_at_end(loop_bb);
    let i_phi = builder.build_phi(i32_ty, "i").or_llvm_err()?;
    let i_val = i_phi.as_basic_value().into_int_value();
    let in_bounds = builder
        .build_int_compare(
            IntPredicate::ULT,
            i_val,
            i32_ty.const_int(pair_count as u64, false),
            "in_bounds",
        )
        .or_llvm_err()?;
    builder
        .build_conditional_branch(in_bounds, loop_body_bb, not_found_bb)
        .or_llvm_err()?;

    // loop_body: load pairs[i*2 + search_offset], compare with cp
    builder.position_at_end(loop_body_bb);
    let i_x2 = builder
        .build_int_mul(i_val, i32_ty.const_int(2, false), "i_x2")
        .or_llvm_err()?;
    let search_idx = builder
        .build_int_add(i_x2, direction, "search_idx")
        .or_llvm_err()?;
    let search_ptr = unsafe {
        builder
            .build_in_bounds_gep(i32_ty, pairs_ptr, &[search_idx], "search_ptr")
            .or_llvm_err()?
    };
    let search_val = builder
        .build_load(i32_ty, search_ptr, "search_val")
        .or_llvm_err()?
        .into_int_value();
    let matches = builder
        .build_int_compare(IntPredicate::EQ, cp_i32, search_val, "matches")
        .or_llvm_err()?;
    builder
        .build_conditional_branch(matches, found_bb, inc_bb)
        .or_llvm_err()?;

    // found: load the other column and return
    builder.position_at_end(found_bb);
    let result_idx = builder
        .build_int_add(i_x2, result_offset, "result_idx")
        .or_llvm_err()?;
    let result_ptr = unsafe {
        builder
            .build_in_bounds_gep(i32_ty, pairs_ptr, &[result_idx], "result_ptr")
            .or_llvm_err()?
    };
    let result_val = builder
        .build_load(i32_ty, result_ptr, "result_val")
        .or_llvm_err()?
        .into_int_value();
    let result_i64 = builder
        .build_int_z_extend(result_val, i64_ty, "result_i64")
        .or_llvm_err()?;
    builder.build_return(Some(&result_i64)).or_llvm_err()?;

    // inc: i++
    builder.position_at_end(inc_bb);
    let i_next = builder
        .build_int_add(i_val, i32_ty.const_int(1, false), "i_next")
        .or_llvm_err()?;
    builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

    // not_found: return cp unchanged
    builder.position_at_end(not_found_bb);
    builder.build_return(Some(&cp)).or_llvm_err()?;

    // Wire phi
    i_phi.add_incoming(&[(&i32_ty.const_zero(), table_scan_bb), (&i_next, inc_bb)]);

    Ok(func)
}

// ---------------------------------------------------------------------------
// UTF-8 decode — replaces verum_utf8_decode_char in C runtime
// ---------------------------------------------------------------------------

/// Emit inline LLVM IR for UTF-8 decode at the current builder position.
///
/// Takes `(ptr bytes, i64 idx)` and returns `i64` codepoint.
/// Uses conditional branches for 1/2/3/4-byte sequences.
/// Returns 0xFFFD for invalid lead bytes.
pub fn emit_utf8_decode_inline<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    bytes_ptr: PointerValue<'ctx>,
    idx_val: IntValue<'ctx>,
) -> Result<IntValue<'ctx>> {
    let context = ctx.llvm_context();
    let i8_ty = context.i8_type();
    let i64_ty = context.i64_type();
    let builder = ctx.builder();

    // Load b0 = bytes[idx] as u8
    let b0_ptr = unsafe {
        builder.build_in_bounds_gep(i8_ty, bytes_ptr, &[idx_val], "b0_ptr")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
    };
    let b0 = builder.build_load(i8_ty, b0_ptr, "b0")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        .into_int_value();
    let b0_64 = builder.build_int_z_extend(b0, i64_ty, "b0_64")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // Helper: load bytes[idx + offset] & 0x3F as i64
    let load_cont = |offset: u64, name: &str| -> Result<IntValue<'ctx>> {
        let off = i64_ty.const_int(offset, false);
        let idx_plus = builder.build_int_add(idx_val, off, &format!("{}_idx", name))
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        let ptr = unsafe {
            builder.build_in_bounds_gep(i8_ty, bytes_ptr, &[idx_plus], &format!("{}_ptr", name))
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
        };
        let byte = builder.build_load(i8_ty, ptr, name)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
            .into_int_value();
        let byte_64 = builder.build_int_z_extend(byte, i64_ty, &format!("{}_64", name))
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        let masked = builder.build_and(byte_64, i64_ty.const_int(0x3F, false), &format!("{}_masked", name))
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        Ok(masked)
    };

    let current_fn = ctx.function();

    // Basic blocks
    let ascii_bb = context.append_basic_block(current_fn, "utf8_ascii");
    let check_2_bb = context.append_basic_block(current_fn, "utf8_check2");
    let decode_2_bb = context.append_basic_block(current_fn, "utf8_2byte");
    let check_3_bb = context.append_basic_block(current_fn, "utf8_check3");
    let decode_3_bb = context.append_basic_block(current_fn, "utf8_3byte");
    let check_4_bb = context.append_basic_block(current_fn, "utf8_check4");
    let decode_4_bb = context.append_basic_block(current_fn, "utf8_4byte");
    let replacement_bb = context.append_basic_block(current_fn, "utf8_replacement");
    let merge_bb = context.append_basic_block(current_fn, "utf8_merge");

    // if b0 < 0x80: ASCII
    let is_ascii = builder.build_int_compare(
        IntPredicate::ULT, b0_64, i64_ty.const_int(0x80, false), "is_ascii")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    builder.build_conditional_branch(is_ascii, ascii_bb, check_2_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // ASCII: return b0
    builder.position_at_end(ascii_bb);
    builder.build_unconditional_branch(merge_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // Check 2-byte: (b0 & 0xE0) == 0xC0
    builder.position_at_end(check_2_bb);
    let masked_e0 = builder.build_and(b0_64, i64_ty.const_int(0xE0, false), "masked_e0")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let is_2byte = builder.build_int_compare(
        IntPredicate::EQ, masked_e0, i64_ty.const_int(0xC0, false), "is_2byte")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    builder.build_conditional_branch(is_2byte, decode_2_bb, check_3_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // 2-byte: ((b0 & 0x1F) << 6) | (b1 & 0x3F)
    builder.position_at_end(decode_2_bb);
    let b0_1f = builder.build_and(b0_64, i64_ty.const_int(0x1F, false), "b0_1f")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b0_shifted = builder.build_left_shift(b0_1f, i64_ty.const_int(6, false), "b0_sh6")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b1_masked = load_cont(1, "b1")?;
    let result_2 = builder.build_or(b0_shifted, b1_masked, "cp_2byte")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    builder.build_unconditional_branch(merge_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // Check 3-byte: (b0 & 0xF0) == 0xE0
    builder.position_at_end(check_3_bb);
    let masked_f0 = builder.build_and(b0_64, i64_ty.const_int(0xF0, false), "masked_f0")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let is_3byte = builder.build_int_compare(
        IntPredicate::EQ, masked_f0, i64_ty.const_int(0xE0, false), "is_3byte")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    builder.build_conditional_branch(is_3byte, decode_3_bb, check_4_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // 3-byte: ((b0 & 0x0F) << 12) | ((b1 & 0x3F) << 6) | (b2 & 0x3F)
    builder.position_at_end(decode_3_bb);
    let b0_0f = builder.build_and(b0_64, i64_ty.const_int(0x0F, false), "b0_0f")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b0_sh12 = builder.build_left_shift(b0_0f, i64_ty.const_int(12, false), "b0_sh12")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b1_3 = load_cont(1, "b1_3")?;
    let b1_sh6 = builder.build_left_shift(b1_3, i64_ty.const_int(6, false), "b1_sh6")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b2_masked = load_cont(2, "b2")?;
    let tmp_3 = builder.build_or(b0_sh12, b1_sh6, "tmp_3a")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let result_3 = builder.build_or(tmp_3, b2_masked, "cp_3byte")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    builder.build_unconditional_branch(merge_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // Check 4-byte: (b0 & 0xF8) == 0xF0
    builder.position_at_end(check_4_bb);
    let masked_f8 = builder.build_and(b0_64, i64_ty.const_int(0xF8, false), "masked_f8")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let is_4byte = builder.build_int_compare(
        IntPredicate::EQ, masked_f8, i64_ty.const_int(0xF0, false), "is_4byte")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    builder.build_conditional_branch(is_4byte, decode_4_bb, replacement_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // 4-byte: ((b0 & 0x07) << 18) | ((b1 & 0x3F) << 12) | ((b2 & 0x3F) << 6) | (b3 & 0x3F)
    builder.position_at_end(decode_4_bb);
    let b0_07 = builder.build_and(b0_64, i64_ty.const_int(0x07, false), "b0_07")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b0_sh18 = builder.build_left_shift(b0_07, i64_ty.const_int(18, false), "b0_sh18")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b1_4 = load_cont(1, "b1_4")?;
    let b1_sh12_4 = builder.build_left_shift(b1_4, i64_ty.const_int(12, false), "b1_sh12_4")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b2_4 = load_cont(2, "b2_4")?;
    let b2_sh6_4 = builder.build_left_shift(b2_4, i64_ty.const_int(6, false), "b2_sh6_4")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let b3_masked = load_cont(3, "b3")?;
    let tmp_4a = builder.build_or(b0_sh18, b1_sh12_4, "tmp_4a")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let tmp_4b = builder.build_or(tmp_4a, b2_sh6_4, "tmp_4b")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    let result_4 = builder.build_or(tmp_4b, b3_masked, "cp_4byte")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    builder.build_unconditional_branch(merge_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // Replacement: return 0xFFFD
    builder.position_at_end(replacement_bb);
    let replacement = i64_ty.const_int(0xFFFD, false);
    builder.build_unconditional_branch(merge_bb)
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

    // Merge with phi
    builder.position_at_end(merge_bb);
    let phi = builder.build_phi(i64_ty, "utf8_cp")
        .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
    phi.add_incoming(&[
        (&b0_64, ascii_bb),
        (&result_2, decode_2_bb),
        (&result_3, decode_3_bb),
        (&result_4, decode_4_bb),
        (&replacement, replacement_bb),
    ]);

    Ok(phi.as_basic_value().into_int_value())
}
