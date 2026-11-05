//! Range cursor state is independent of every valid integer and of sibling iterators.
use std::sync::Arc;
use verum_vbc::{bytecode, instruction::{Instruction as I, Reg, CompareOp}, interpreter::Interpreter,
    module::{FunctionDescriptor, FunctionId, VbcModule}, types::StringId};

fn run(start: i64, end: i64, inclusive: bool, expected: &[i64]) {
    let mut module = VbcModule::new("range_cursor".into());
    let message = module.strings.intern("range cursor mismatch");
    let mut code = vec![I::LoadI{dst:Reg(0),value:start}, I::LoadI{dst:Reg(1),value:end},
        I::NewRange{dst:Reg(2),start:Reg(0),end:Reg(1),inclusive},
        I::IterNew{dst:Reg(3),iterable:Reg(2)}, I::IterNew{dst:Reg(4),iterable:Reg(2)}];
    // Complete the first iterator; the second still begins at the original start.
    for iter in [3,4] {
        for expected in expected {
            code.extend([I::IterNext{dst:Reg(5),has_next:Reg(6),iter:Reg(iter)},
                I::Assert{cond:Reg(6),message_id:message.0}, I::LoadI{dst:Reg(7),value:*expected},
                I::CmpI{op:CompareOp::Eq,dst:Reg(8),a:Reg(5),b:Reg(7)},
                I::Assert{cond:Reg(8),message_id:message.0}]);
        }
        for _ in 0..3 {
            code.extend([I::IterNext{dst:Reg(5),has_next:Reg(6),iter:Reg(iter)},
                I::Not{dst:Reg(7),src:Reg(6)}, I::Assert{cond:Reg(7),message_id:message.0}]);
        }
    }
    code.extend([I::LoadI{dst:Reg(0),value:42},I::Ret{value:Reg(0)}]);
    for op in code { bytecode::encode_instruction(&op, &mut module.bytecode); }
    let mut function = FunctionDescriptor::new(StringId::EMPTY);
    function.id = FunctionId(0);function.bytecode_length = module.bytecode.len() as u32;function.register_count=9;
    module.functions.push(function);
    let mut vm = Interpreter::new(Arc::new(module));
    assert_eq!(vm.execute_function(FunctionId(0)).unwrap().as_i64(),42);
}
#[test] fn negative_to_positive_and_fused_exhaustion(){run(-1,1,true,&[-1,0,1]);}
#[test] fn maximum_and_fused_exhaustion(){run(i64::MAX,i64::MAX,true,&[i64::MAX]);}
#[test] fn exclusive_singleton_is_empty(){run(0,0,false,&[]);}
#[test] fn reversed_range_stays_empty(){run(1,-1,true,&[]);}
#[test] fn empty_at_maximum_stays_empty(){run(i64::MAX,i64::MAX,false,&[]);}
#[test] fn minimum_is_an_ordinary_cursor(){run(i64::MIN,i64::MIN+1,true,&[i64::MIN,i64::MIN+1]);}
