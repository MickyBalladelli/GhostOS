use crate::InspectError;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Name<const CAPACITY: usize = 64> {
    bytes: [u8; CAPACITY],
    length: u8,
}

impl<const CAPACITY: usize> Name<CAPACITY> {
    pub fn new(value: &str) -> Result<Self, InspectError> {
        if value.is_empty() || value.len() > CAPACITY || value.len() > u8::MAX as usize {
            return Err(InspectError::InvalidSample)
        }
        let mut bytes = [0; CAPACITY];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize])
            .expect("inspection name invariant")
    }
}

impl<const CAPACITY: usize> core::fmt::Debug for Name<CAPACITY> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_tuple("Name").field(&self.as_str()).finish()
    }
}
