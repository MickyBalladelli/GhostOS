pub(crate) fn limits(max_source_bytes: usize, max_operations: u64, max_call_levels: usize,
    max_expression_depth: usize, max_variables: usize, max_functions: usize, max_modules: usize,
    max_string_bytes: usize, max_array_items: usize, max_map_items: usize, max_requests: usize,
    max_request_bytes: usize) -> bool {
    unsafe { ghostos_embedded_limits(max_source_bytes, max_operations, max_call_levels,
        max_expression_depth, max_variables, max_functions, max_modules, max_string_bytes,
        max_array_items, max_map_items, max_requests, max_request_bytes) }
}
pub(crate) fn capability(name: &[u8], operations: u64) -> bool {
    unsafe { ghostos_embedded_capability(name.as_ptr(), name.len(), operations) == 0 }
}
pub(crate) fn operation_mask(operation: u8) -> Option<u64> {
    let mut mask = 0;
    if unsafe { ghostos_embedded_operation_mask(operation, &mut mask) } { Some(mask) } else { None }
}
pub(crate) fn allows(operations: u64, operation: u8) -> bool {
    unsafe { ghostos_embedded_allows(operations, operation) }
}
pub(crate) fn names(names: &[*const u8], lengths: &[usize], count: usize, capacity: usize) -> i32 {
    unsafe { ghostos_embedded_names(names.as_ptr(), lengths.as_ptr(), count, capacity) }
}
pub(crate) fn source(length: usize, max_source_bytes: usize) -> bool {
    unsafe { ghostos_embedded_source(length, max_source_bytes) }
}
pub(crate) fn request(operation_ok: bool, operation: u8, allowed: bool, payload_length: usize,
    max_payload: usize, requests: usize, max_requests: usize) -> Result<u32, i32> {
    let mut sequence = 0;
    let code = unsafe { ghostos_embedded_request(operation_ok, operation, allowed, payload_length,
        max_payload, requests, max_requests, &mut sequence) };
    if code == 0 { Ok(sequence) } else { Err(code) }
}
unsafe extern "C" {
    fn ghostos_embedded_limits(max_source_bytes: usize, max_operations: u64, max_call_levels: usize,
        max_expression_depth: usize, max_variables: usize, max_functions: usize, max_modules: usize,
        max_string_bytes: usize, max_array_items: usize, max_map_items: usize, max_requests: usize,
        max_request_bytes: usize) -> bool;
    fn ghostos_embedded_capability(name: *const u8, length: usize, operations: u64) -> i32;
    fn ghostos_embedded_operation_mask(operation: u8, mask: *mut u64) -> bool;
    fn ghostos_embedded_allows(operations: u64, operation: u8) -> bool;
    fn ghostos_embedded_names(names: *const *const u8, lengths: *const usize, count: usize, capacity: usize) -> i32;
    fn ghostos_embedded_source(length: usize, max_source_bytes: usize) -> bool;
    fn ghostos_embedded_request(operation_ok: bool, operation: u8, allowed: bool, payload_length: usize,
        max_payload: usize, requests: usize, max_requests: usize, sequence: *mut u32) -> i32;
}
