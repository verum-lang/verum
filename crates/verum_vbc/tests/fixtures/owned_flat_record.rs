#[path = "canonical_drop.rs"]
mod canonical_drop;
#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{ItemFailurePolicy, VbcCodegen},
};

pub fn source_module() -> VbcModule {
    let source = r#"
        mount core.base.protocols.Drop;
        mount core.intrinsics.memory.{ptr_write, ptr_read};
        static mut RELEASED: Int = 0;
        type affine Ticket is { number: Int };
        type affine Envelope is { ticket: Ticket, sequence: Int };
        implement Drop for Ticket {
            fn drop(&mut self) { unsafe { RELEASED = RELEASED + self.number; } }
        }
        fn store(address: *mut Envelope, number: Int) {
            let owner = Envelope { ticket: Ticket { number: number }, sequence: 7 };
            unsafe { ptr_write(address, owner); }
        }
        fn take(address: *mut Envelope) -> Int {
            let owner: Envelope = unsafe { ptr_read(address) };
            owner.ticket.number * 10 + owner.sequence
        }
        fn transfer(address: *mut Envelope, owner: Envelope) {
            unsafe { ptr_write(address, owner); }
        }
        fn make_owner() -> Envelope {
            Envelope { ticket: Ticket { number: 31 }, sequence: 17 }
        }
        fn releases() -> Int { unsafe { RELEASED } }
        type Pair is { left: Int, right: Int };
        type Note is { pair: Pair, label: Int };
        fn copy_note(address: *mut Note) -> Int {
            let mut original = Note { pair: Pair { left: 17, right: 19 }, label: 5 };
            unsafe { ptr_write(address, original); }
            original.pair.left = 37;
            let stored: Note = unsafe { ptr_read(address) };
            if stored.pair.left == 17 && stored.pair.right == 19 && stored.label == 5 {
                original.pair.left
            } else { -1 }
        }
        fn main() -> Int {
            let raw = unsafe { @intrinsic("alloc", 16, 8) };
            let address = raw as *mut Envelope;
            store(address, 13);
            if releases() != 0 || take(address) != 137 || releases() != 13 { return 1; }
            store(address, 29);
            if releases() != 13 || take(address) != 297 || releases() != 42 { return 2; }
            if copy_note(raw as *mut Note) != 37 { return 3; }
            unsafe { @intrinsic("dealloc", raw, 16, 8); }
            73
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let mut generator = VbcCodegen::new();
    canonical_drop::initialize(&mut generator);
    let memory = selected_stdlib::source_unit("intrinsics/memory.vr", "ptr_read ptr_write", 1);
    let units = [&memory, &ast];
    generator.collect_unit_declarations(&units).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    generator.finalize_module().unwrap()
}
