; Each thread owns a list of context bindings. A frame marker starts an
; inherited overlay. Reads cross it, writes and scope ends stay above it.
; Entry kinds are flat=0, lexical=1, frame=2, explicit provide=3.
; Bit 2 masks a binding cleared inside its scope.
%context.entry = type { ptr, i64, i64, i64 }
@context.head = internal thread_local global ptr null
@context.depth = internal thread_local global i64 0

declare ptr @verum_alloc(i64)
declare void @verum_dealloc(ptr, i64)
declare void @verum_internal_exit_i64(i64) noreturn

define internal ptr @context.push(i64 %key, i64 %value, i64 %kind) {
entry:
  %node = call ptr @verum_alloc(i64 32)
  %failed = icmp eq ptr %node, null
  br i1 %failed, label %oom, label %publish
oom:
  call void @verum_internal_exit_i64(i64 1)
  unreachable
publish:
  %head = load ptr, ptr @context.head
  store ptr %head, ptr %node
  %k = getelementptr %context.entry, ptr %node, i32 0, i32 1
  %v = getelementptr %context.entry, ptr %node, i32 0, i32 2
  %t = getelementptr %context.entry, ptr %node, i32 0, i32 3
  store i64 %key, ptr %k
  store i64 %value, ptr %v
  store i64 %kind, ptr %t
  store ptr %node, ptr @context.head
  ret ptr %node
}

; Return the first matching binding, including an explicitly masked binding.
define internal ptr @context.find(i64 %key, i1 %local) {
entry:
  %head = load ptr, ptr @context.head
  br label %scan
scan:
  %node = phi ptr [ %head, %entry ], [ %previous, %next ]
  %empty = icmp eq ptr %node, null
  br i1 %empty, label %absent, label %test
test:
  %t = getelementptr %context.entry, ptr %node, i32 0, i32 3
  %kind = load i64, ptr %t
  %boundary = icmp eq i64 %kind, 2
  br i1 %boundary, label %frame, label %keyed
frame:
  br i1 %local, label %absent, label %next
keyed:
  %k = getelementptr %context.entry, ptr %node, i32 0, i32 1
  %candidate = load i64, ptr %k
  %matches = icmp eq i64 %candidate, %key
  br i1 %matches, label %found, label %next
next:
  %previous = load ptr, ptr %node
  br label %scan
found:
  ret ptr %node
absent:
  ret ptr null
}

define internal i64 @context.read(i64 %key, i1 %presence) {
entry:
  %node = call ptr @context.find(i64 %key, i1 false)
  %empty = icmp eq ptr %node, null
  br i1 %empty, label %absent, label %test
test:
  %t = getelementptr %context.entry, ptr %node, i32 0, i32 3
  %kind = load i64, ptr %t
  %live = icmp ult i64 %kind, 4
  br i1 %live, label %found, label %absent
found:
  %v = getelementptr %context.entry, ptr %node, i32 0, i32 2
  %value = load i64, ptr %v
  %result = select i1 %presence, i64 1, i64 %value
  ret i64 %result
absent:
  ret i64 0
}

define i64 @verum_ctx_get(i32 %slot) {
  %key = zext i32 %slot to i64
  %value = call i64 @context.read(i64 %key, i1 false)
  ret i64 %value
}
define i64 @verum_ctx_has_binding(i32 %slot) {
  %key = zext i32 %slot to i64
  %present = call i64 @context.read(i64 %key, i1 true)
  ret i64 %present
}
define void @verum_ctx_provide(i32 %slot, i64 %value, i64 %depth) {
  %key = zext i32 %slot to i64
  call ptr @context.push(i64 %key, i64 %value, i64 1)
  ret void
}

; Remove an entry by updating its predecessor link. A lexical end skips
; manual slots inserted during the scope, even after a clear of that scope.
define internal void @context.end(i64 %key, i8 %mode) {
entry:
  br label %scan
scan:
  %link = phi ptr [ @context.head, %entry ], [ %node, %next ]
  %node = load ptr, ptr %link
  %empty = icmp eq ptr %node, null
  br i1 %empty, label %done, label %test
test:
  %t = getelementptr %context.entry, ptr %node, i32 0, i32 3
  %kind = load i64, ptr %t
  %boundary = icmp eq i64 %kind, 2
  br i1 %boundary, label %done, label %keyed
keyed:
  %base = and i64 %kind, 3
  %lexical = icmp eq i64 %base, 1
  %k = getelementptr %context.entry, ptr %node, i32 0, i32 1
  %candidate = load i64, ptr %k
  %same_key = icmp eq i64 %candidate, %key
  %lexical_key = and i1 %lexical, %same_key
  %any_keyed = icmp eq i8 %mode, 1
  %keyed_match = select i1 %any_keyed, i1 %same_key, i1 %lexical_key
  %innermost = icmp eq i8 %mode, 0
  %matches = select i1 %innermost, i1 %lexical, i1 %keyed_match
  br i1 %matches, label %erase, label %next
next:
  br label %scan
erase:
  %previous = load ptr, ptr %node
  store ptr %previous, ptr %link
  call void @verum_dealloc(ptr %node, i64 32)
  br label %done
done:
  ret void
}
define void @verum_ctx_end(i64 %depth) {
  call void @context.end(i64 0, i8 0)
  ret void
}

define void @verum_ctx_end_named(i32 %name) {
  %key = zext i32 %name to i64
  call void @context.end(i64 %key, i8 2)
  ret void
}

define i64 @verum_ctx_slot_get(i64 %slot) {
  %valid = icmp ult i64 %slot, 256
  br i1 %valid, label %read, label %empty
read:
  %v = call i64 @context.read(i64 %slot, i1 false)
  ret i64 %v
empty:
  ret i64 0
}

define i64 @verum_ctx_slot_set(i64 %slot, i64 %value) {
entry:
  %null = icmp eq i64 %value, 0
  br i1 %null, label %clear, label %test_range
clear:
  call i64 @verum_ctx_slot_clear(i64 %slot)
  ret i64 0
test_range:
  %valid = icmp ult i64 %slot, 256
  br i1 %valid, label %local, label %done
local:
  %node = call ptr @context.find(i64 %slot, i1 true)
  %absent = icmp eq ptr %node, null
  br i1 %absent, label %insert, label %test
test:
  %t = getelementptr %context.entry, ptr %node, i32 0, i32 3
  %kind = load i64, ptr %t
  %base = and i64 %kind, 3
  %scoped = icmp eq i64 %base, 1
  br i1 %scoped, label %insert, label %update
insert:
  call ptr @context.push(i64 %slot, i64 %value, i64 0)
  br label %done
update:
  %v = getelementptr %context.entry, ptr %node, i32 0, i32 2
  store i64 %base, ptr %t
  store i64 %value, ptr %v
  br label %done
done:
  ret i64 0
}

define i64 @verum_ctx_slot_clear(i64 %slot) {
entry:
  %valid = icmp ult i64 %slot, 256
  br i1 %valid, label %scan, label %done
scan:
  %link = phi ptr [ @context.head, %entry ], [ %node, %next ], [ %link, %erase ]
  %node = load ptr, ptr %link
  %empty = icmp eq ptr %node, null
  br i1 %empty, label %done, label %test
test:
  %t = getelementptr %context.entry, ptr %node, i32 0, i32 3
  %kind = load i64, ptr %t
  %boundary = icmp eq i64 %kind, 2
  br i1 %boundary, label %inherited, label %keyed
keyed:
  %k = getelementptr %context.entry, ptr %node, i32 0, i32 1
  %candidate = load i64, ptr %k
  %same = icmp eq i64 %candidate, %slot
  br i1 %same, label %clear, label %next
clear:
  %base = and i64 %kind, 3
  %lexical = icmp eq i64 %base, 1
  br i1 %lexical, label %mask, label %erase
mask:
  store i64 5, ptr %t
  br label %next
next:
  br label %scan
erase:
  %previous = load ptr, ptr %node
  store ptr %previous, ptr %link
  call void @verum_dealloc(ptr %node, i64 32)
  br label %scan
inherited:
  call ptr @context.push(i64 %slot, i64 0, i64 4)
  br label %done
done:
  ret i64 0
}

define i64 @verum_ctx_frame_push() {
  %depth = load i64, ptr @context.depth
  %room = icmp ult i64 %depth, 8
  br i1 %room, label %push, label %overflow
push:
  %mark = call ptr @context.push(i64 0, i64 0, i64 2)
  %next = add i64 %depth, 1
  store i64 %next, ptr @context.depth
  %token = ptrtoint ptr %mark to i64
  ret i64 %token
overflow:
  ret i64 0
}

define i64 @verum_ctx_frame_pop() {
entry:
  %depth = load i64, ptr @context.depth
  %empty = icmp eq i64 %depth, 0
  br i1 %empty, label %underflow, label %pop
pop:
  %node = load ptr, ptr @context.head
  %previous = load ptr, ptr %node
  %t = getelementptr %context.entry, ptr %node, i32 0, i32 3
  %kind = load i64, ptr %t
  %boundary = icmp eq i64 %kind, 2
  store ptr %previous, ptr @context.head
  call void @verum_dealloc(ptr %node, i64 32)
  br i1 %boundary, label %done, label %pop
done:
  %next = sub i64 %depth, 1
  store i64 %next, ptr @context.depth
  ret i64 1
underflow:
  ret i64 0
}

; Existing dynamic context functions accept keys beyond the fixed slot table.
define i64 @verum_ctx_explicit_get(i64 %name) {
  %key = and i64 %name, 2147483647
  %value = call i64 @context.read(i64 %key, i1 false)
  ret i64 %value
}
define i64 @verum_ctx_explicit_provide(i64 %name, i64 %value) {
  %key = and i64 %name, 2147483647
  call ptr @context.push(i64 %key, i64 %value, i64 3)
  ret i64 0
}
define i64 @verum_ctx_explicit_end(i64 %name) {
  %key = and i64 %name, 2147483647
  call void @context.end(i64 %key, i8 1)
  ret i64 0
}
