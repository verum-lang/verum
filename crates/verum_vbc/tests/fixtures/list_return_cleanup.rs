//! Parsed callable-return fixtures shared by interpreter and native controls.

pub const BLOCK_SIGNED: &str = "fn probe() -> List<Int16> { let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; values }";
pub const BLOCK_UNSIGNED: &str = "fn probe() -> List<UInt16> { let values: [UInt16; 3] = [32768, 65535, 1]; let first = values[0]; values }";
pub const EXPLICIT: &str = "fn probe() -> List<Int16> { let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; return values; }";
pub const EXPLICIT_BLOCK: &str = "fn probe() -> List<Int16> { return { let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; values }; }";
pub const EXPRESSION_BODY: &str = "fn probe() -> List<Int16> = { let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; values };";
pub const NESTED_TAIL: &str = "fn probe() -> List<Int16> { ({ let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; values }) }";
pub const BRANCH_TAIL: &str = "fn probe() -> List<Int16> { if true { let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; values } else { let values: [Int16; 3] = [-2, -3, 1]; let first = values[0]; values } }";
pub const LIST_CLOSURE: &str = "fn probe() -> List<Int16> { let build = || -> List<Int16> { let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; values }; build() }";
// This isolates return-context storage. The initial signed indirect result
// exposed a separate element-type propagation failure, retained in the baseline.
pub const ARRAY_CLOSURE: &str = "fn probe() -> List<UInt16> { let build = || -> [UInt16; 3] { let values: [UInt16; 3] = [32768, 65535, 1]; values }; let values = build(); assert_eq(values[0], 32768); [32768, 65535, 1] }";

pub const OWNED_CLEANUP: &str = r#"
type Count is { value: Int };
type affine Watch is { counter: &mut Count, digit: Int };
implement Drop for Watch {
    fn drop(&mut self) { self.counter.value = self.counter.value * 10 + self.digit; }
}
fn make(counter: &mut Count) -> List<Int16> {
    let first_guard: Watch = Watch { counter, digit: 1 };
    let values: [Int16; 3] = [-32768, -1, 1];
    let last_guard: Watch = Watch { counter, digit: 2 };
    let first = values[0];
    defer { counter.value = 3; }
    values
}
fn probe() -> List<Int16> {
    let mut counter = Count { value: 0 };
    let values = make(&mut counter);
    assert_eq(counter.value, 321);
    values
}
"#;

pub const BORROWED_OWNER: &str = r#"
type Count is { value: Int };
type affine Watch is { counter: &mut Count, digit: Int };
implement Drop for Watch {
    fn drop(&mut self) { self.counter.value = self.counter.value * 10 + self.digit; }
}
fn make(counter: &mut Count, owner: &Watch) -> List<Int16> {
    let values: [Int16; 3] = [-32768, -1, 1];
    let borrowed: &Watch = owner;
    let first = values[0];
    values
}
fn probe() -> List<Int16> {
    let mut counter = Count { value: 0 };
    let mut result: List<Int16> = [];
    {
        let owner: Watch = Watch { counter: &mut counter, digit: 7 };
        result = make(&mut counter, &owner);
        assert_eq(counter.value, 0);
    }
    assert_eq(counter.value, 7);
    result
}
"#;
