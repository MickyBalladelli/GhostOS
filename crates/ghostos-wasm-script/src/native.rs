pub(crate) fn limits(fuel: u64, max_module_bytes: usize, max_memory_bytes: usize,
    max_table_elements: usize, max_instances: usize, max_memories: usize, max_tables: usize) -> bool {
    unsafe { ghostos_wasm_limits(fuel, max_module_bytes, max_memory_bytes, max_table_elements,
        max_instances, max_memories, max_tables) }
}
pub(crate) fn module(wasm_len: usize, max_module_bytes: usize, entry_len: usize) -> i32 {
    unsafe { ghostos_wasm_module(wasm_len, max_module_bytes, entry_len) }
}
pub(crate) fn allows(operations: u64, operation: u8) -> bool {
    unsafe { ghostos_wasm_allows(operations, operation) }
}
pub(crate) fn grants(handles: &[u64], operations: &[u64], count: usize, capacity: usize) -> i32 {
    unsafe { ghostos_wasm_grants(handles.as_ptr(), operations.as_ptr(), count, capacity) }
}
pub(crate) fn find(handles: &[u64], operations: &[u64], occupied: &[bool], handle: u64) -> Option<u64> {
    let mut found = 0;
    if unsafe { ghostos_wasm_find(handles.as_ptr(), operations.as_ptr(), occupied.as_ptr(),
        handles.len(), handle, &mut found) } { Some(found) } else { None }
}
pub(crate) fn check(operation_ok: bool, allows: bool) -> i32 {
    unsafe { ghostos_wasm_check(operation_ok, allows) }
}
pub(crate) fn invoke(operation_ok: bool, found: bool, allows: bool) -> i32 {
    unsafe { ghostos_wasm_invoke(operation_ok, found, allows) }
}
pub(crate) fn fuel_consumed(fuel: u64, remaining: u64) -> u64 {
    unsafe { ghostos_wasm_fuel_consumed(fuel, remaining) }
}
unsafe extern "C" {
    fn ghostos_wasm_limits(fuel: u64, max_module_bytes: usize, max_memory_bytes: usize,
        max_table_elements: usize, max_instances: usize, max_memories: usize, max_tables: usize) -> bool;
    fn ghostos_wasm_module(wasm_len: usize, max_module_bytes: usize, entry_len: usize) -> i32;
    fn ghostos_wasm_allows(operations: u64, operation: u8) -> bool;
    fn ghostos_wasm_grants(handles: *const u64, operations: *const u64, count: usize, capacity: usize) -> i32;
    fn ghostos_wasm_find(handles: *const u64, operations: *const u64, occupied: *const bool,
        count: usize, handle: u64, found_operations: *mut u64) -> bool;
    fn ghostos_wasm_check(operation_ok: bool, allows: bool) -> i32;
    fn ghostos_wasm_invoke(operation_ok: bool, found: bool, allows: bool) -> i32;
    fn ghostos_wasm_fuel_consumed(fuel: u64, remaining: u64) -> u64;
}
