; Target-independent control flow for SysExtended.LinuxSignalWatch.
; Only exchange_action and observer_action are supplied by the target emitter.
%action = type { ptr, i64, ptr, i64 }
%slot = type { i8, i64, i64, %action } ; control lock, signed ticket, pending, saved action
@"verum#linux.signal.slots.v1" = linkonce_odr hidden global [64 x %slot] zeroinitializer, align 8
@observer_action = external constant %action
declare i64 @exchange_action(i64, ptr, ptr)

; Asynchronous entry from the kernel. No allocation, calls or locks.
define internal void @count_delivery(i32 %sig) nounwind {
entry:
  %i = sub i32 %sig, 1
  %valid = icmp ult i32 %i, 64
  br i1 %valid, label %load, label %done
load:
  %pending = getelementptr inbounds [64 x %slot], ptr @"verum#linux.signal.slots.v1", i64 0, i32 %i, i32 2
  %first = load atomic i64, ptr %pending monotonic, align 8
  br label %retry
retry:
  %n = phi i64 [ %first, %load ], [ %observed, %inc ]
  %full = icmp slt i64 %n, 0
  br i1 %full, label %done, label %inc
inc:
  %next = add i64 %n, 1
  %cas = cmpxchg weak ptr %pending, i64 %n, i64 %next monotonic monotonic
  %observed = extractvalue { i64, i1 } %cas, 0
  %updated = extractvalue { i64, i1 } %cas, 1
  br i1 %updated, label %done, label %retry
done:
  ret void
}

define i64 @watch(i8 %op, i64 %value) nounwind {
entry:
  %install = icmp eq i8 %op, 0
  %positive = icmp sgt i64 %value, 0
  %too_high = icmp ugt i64 %value, 64
  %bad_signal = and i1 %install, %too_high
  %bad_op = icmp ugt i8 %op, 2
  %invalid = or i1 %bad_signal, %bad_op
  %valid_range = xor i1 %invalid, true
  %valid = and i1 %positive, %valid_range
  br i1 %valid, label %lock, label %invalid_arg
invalid_arg:
  ret i64 -22
lock:
  %zero_based = sub i64 %value, 1
  %i = and i64 %zero_based, 63
  %signal = add i64 %i, 1
  %slot_ptr = getelementptr inbounds [64 x %slot], ptr @"verum#linux.signal.slots.v1", i64 0, i64 %i
  %claim = cmpxchg ptr %slot_ptr, i8 0, i8 1 acquire monotonic
  %locked = extractvalue { i8, i1 } %claim, 1
  br i1 %locked, label %inspect, label %busy
busy:
  ret i64 -16
inspect:
  %owner = getelementptr inbounds %slot, ptr %slot_ptr, i32 0, i32 1
  %pending = getelementptr inbounds %slot, ptr %slot_ptr, i32 0, i32 2
  %saved = getelementptr inbounds %slot, ptr %slot_ptr, i32 0, i32 3
  %ticket = load i64, ptr %owner
  br i1 %install, label %reserve, label %check_owner
reserve:
  %taken = icmp sgt i64 %ticket, 0
  br i1 %taken, label %unlock, label %generation
generation:
  %last = sub i64 0, %ticket
  %exhausted = icmp ugt i64 %last, 9223372036854775743 ; INT64_MAX - 64
  br i1 %exhausted, label %unlock, label %set
set:
  %first = icmp eq i64 %last, 0
  %next = add i64 %last, 64
  %new = select i1 %first, i64 %signal, i64 %next
  store atomic i64 0, ptr %pending monotonic, align 8
  %rc = call i64 @exchange_action(i64 %signal, ptr @observer_action, ptr %saved)
  %installed = icmp eq i64 %rc, 0
  br i1 %installed, label %own, label %unlock
own:
  store i64 %new, ptr %owner
  br label %unlock
check_owner:
  %mine = icmp eq i64 %ticket, %value
  br i1 %mine, label %dispatch, label %unlock
dispatch:
  %take = icmp eq i8 %op, 1
  br i1 %take, label %drain, label %restore
drain:
  %count = atomicrmw xchg ptr %pending, i64 0 monotonic
  %overflow = icmp slt i64 %count, 0
  %result = select i1 %overflow, i64 -75, i64 %count
  br label %unlock
restore:
  %restored = call i64 @exchange_action(i64 %signal, ptr %saved, ptr null)
  %ok = icmp eq i64 %restored, 0
  %inactive = sub i64 0, %ticket
  %after = select i1 %ok, i64 %inactive, i64 %ticket
  store i64 %after, ptr %owner
  br label %unlock
unlock:
  %answer = phi i64 [ -16, %reserve ], [ -75, %generation ], [ %rc, %set ],
                    [ %new, %own ], [ -9, %check_owner ], [ %result, %drain ],
                    [ %restored, %restore ]
  store atomic i8 0, ptr %slot_ptr release, align 1
  ret i64 %answer
}
