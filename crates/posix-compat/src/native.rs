use crate::{MAX_PSEUDO_PATH_BYTES, PseudoPathError};
pub(crate) fn pseudo_parse(path: &[u8]) -> Result<([u8; MAX_PSEUDO_PATH_BYTES], u8, u8), PseudoPathError> {
    let mut logical = [0; MAX_PSEUDO_PATH_BYTES];
    let mut length = 0;
    let mut kind = 0;
    match unsafe { ghostos_posix_pseudo_parse(path.as_ptr(), path.len(), logical.as_mut_ptr(), &mut length, &mut kind) } {
        0 => Ok((logical, length, kind)),
        1 => Err(PseudoPathError::NotPseudoPath),
        2 => Err(PseudoPathError::InvalidPath),
        3 => Err(PseudoPathError::TooLong),
        _ => unreachable!("native POSIX pseudo path result"),
    }
}
const _: () = assert!(MAX_PSEUDO_PATH_BYTES == 64);
unsafe extern "C" {
    fn ghostos_posix_pseudo_parse(path: *const u8, length: usize,
        logical: *mut u8, logical_length: *mut u8, kind: *mut u8) -> i32;
}
