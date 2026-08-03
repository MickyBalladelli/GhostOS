use synos_ipc::{SharedBuffer, SharedRegionId};

use crate::Error;

pub const MAX_TENSOR_RANK: usize = 8;
pub const MAX_TENSOR_ELEMENTS: u64 = u32::MAX as u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DType {
    Bool,
    U8,
    I8,
    U16,
    I16,
    F16,
    BFloat16,
    U32,
    I32,
    F32,
    U64,
    I64,
    F64,
}

impl DType {
    pub const fn size(self) -> usize {
        match self {
            Self::Bool | Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 | Self::F16 | Self::BFloat16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
        }
    }

    pub const fn alignment(self) -> usize {
        self.size()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TensorShape {
    rank: u8,
    dimensions: [u32; MAX_TENSOR_RANK],
}

impl TensorShape {
    pub fn new(dimensions: &[u32]) -> Result<Self, Error> {
        if dimensions.len() > MAX_TENSOR_RANK || dimensions.contains(&0) {
            return Err(Error::InvalidShape);
        }
        let mut stored = [0; MAX_TENSOR_RANK];
        stored[..dimensions.len()].copy_from_slice(dimensions);
        let shape = Self {
            rank: dimensions.len() as u8,
            dimensions: stored,
        };
        if shape.element_count()? > MAX_TENSOR_ELEMENTS {
            return Err(Error::InvalidShape);
        }
        Ok(shape)
    }

    pub const fn scalar() -> Self {
        Self {
            rank: 0,
            dimensions: [0; MAX_TENSOR_RANK],
        }
    }

    pub const fn rank(self) -> usize {
        self.rank as usize
    }

    pub fn dimensions(&self) -> &[u32] {
        &self.dimensions[..self.rank()]
    }

    pub fn element_count(self) -> Result<u64, Error> {
        self.dimensions().iter().try_fold(1u64, |count, dimension| {
            count
                .checked_mul(*dimension as u64)
                .ok_or(Error::InvalidShape)
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TensorLayout {
    pub dtype: DType,
    pub shape: TensorShape,
    strides: [u64; MAX_TENSOR_RANK],
}

impl TensorLayout {
    pub fn contiguous(dtype: DType, shape: TensorShape) -> Result<Self, Error> {
        let mut strides = [0; MAX_TENSOR_RANK];
        let mut stride = dtype.size() as u64;
        for index in (0..shape.rank()).rev() {
            strides[index] = stride;
            stride = stride
                .checked_mul(shape.dimensions[index] as u64)
                .ok_or(Error::InvalidShape)?
        }
        Ok(Self {
            dtype,
            shape,
            strides,
        })
    }

    pub fn strided(dtype: DType, shape: TensorShape, source: &[u64]) -> Result<Self, Error> {
        if source.len() != shape.rank() {
            return Err(Error::InvalidStride);
        }
        let mut strides = [0; MAX_TENSOR_RANK];
        for (destination, stride) in strides.iter_mut().zip(source) {
            if *stride < dtype.size() as u64 || *stride % dtype.size() as u64 != 0 {
                return Err(Error::InvalidStride);
            }
            *destination = *stride
        }
        let layout = Self {
            dtype,
            shape,
            strides,
        };
        layout.required_bytes()?;
        Ok(layout)
    }

    pub fn strides(&self) -> &[u64] {
        &self.strides[..self.shape.rank()]
    }

    pub fn is_contiguous(self) -> bool {
        Self::contiguous(self.dtype, self.shape)
            .is_ok_and(|contiguous| contiguous.strides == self.strides)
    }

    pub fn required_bytes(self) -> Result<u64, Error> {
        let mut bytes = self.dtype.size() as u64;
        for index in 0..self.shape.rank() {
            let tail = (self.shape.dimensions[index] as u64 - 1)
                .checked_mul(self.strides[index])
                .ok_or(Error::InvalidStride)?;
            bytes = bytes.checked_add(tail).ok_or(Error::InvalidStride)?
        }
        Ok(bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedTensor {
    pub buffer: SharedBuffer,
    pub layout: TensorLayout,
}

impl SharedTensor {
    pub fn new(buffer: SharedBuffer, layout: TensorLayout) -> Result<Self, Error> {
        let tensor = Self { buffer, layout };
        tensor.validate()?;
        Ok(tensor)
    }

    pub fn validate(self) -> Result<Self, Error> {
        let required = self.layout.required_bytes()?;
        let end = (self.buffer.offset as u64)
            .checked_add(required)
            .ok_or(Error::InvalidTensor)?;
        if end > self.buffer.length as u64
            || self.buffer.offset as usize % self.layout.dtype.alignment() != 0
        {
            return Err(Error::InvalidTensor);
        }
        Ok(self)
    }
}

#[derive(Debug)]
pub struct TensorView<'a> {
    bytes: &'a [u8],
    layout: TensorLayout,
}

impl<'a> TensorView<'a> {
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub const fn layout(&self) -> TensorLayout {
        self.layout
    }
}

#[derive(Debug)]
pub struct TensorViewMut<'a> {
    bytes: &'a mut [u8],
    layout: TensorLayout,
}

impl<'a> TensorViewMut<'a> {
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        self.bytes
    }

    pub const fn layout(&self) -> TensorLayout {
        self.layout
    }
}

/// Read-only view over one capability-mapped IPC region.
pub struct TensorRegion<'a> {
    id: SharedRegionId,
    bytes: &'a [u8],
}

impl<'a> TensorRegion<'a> {
    pub const fn new(id: SharedRegionId, bytes: &'a [u8]) -> Self {
        Self { id, bytes }
    }

    pub fn map(&self, tensor: SharedTensor) -> Result<TensorView<'_>, Error> {
        tensor.validate()?;
        if tensor.buffer.region != self.id {
            return Err(Error::RegionMismatch);
        }
        let bytes = resolve(self.bytes, tensor)?;
        Ok(TensorView {
            bytes,
            layout: tensor.layout,
        })
    }
}

/// Writable view over one capability-mapped IPC region.
pub struct TensorRegionMut<'a> {
    id: SharedRegionId,
    bytes: &'a mut [u8],
}

impl<'a> TensorRegionMut<'a> {
    pub fn new(id: SharedRegionId, bytes: &'a mut [u8]) -> Self {
        Self { id, bytes }
    }

    pub fn map(&self, tensor: SharedTensor) -> Result<TensorView<'_>, Error> {
        tensor.validate()?;
        if tensor.buffer.region != self.id {
            return Err(Error::RegionMismatch);
        }
        let bytes = resolve(self.bytes, tensor)?;
        Ok(TensorView {
            bytes,
            layout: tensor.layout,
        })
    }

    pub fn map_mut(&mut self, tensor: SharedTensor) -> Result<TensorViewMut<'_>, Error> {
        tensor.validate()?;
        if tensor.buffer.region != self.id {
            return Err(Error::RegionMismatch);
        }
        if !tensor.buffer.writable {
            return Err(Error::ReadOnly);
        }
        let start = tensor.buffer.offset as usize;
        let length =
            usize::try_from(tensor.layout.required_bytes()?).map_err(|_| Error::InvalidTensor)?;
        let end = start.checked_add(length).ok_or(Error::InvalidTensor)?;
        let bytes = self
            .bytes
            .get_mut(start..end)
            .ok_or(Error::BufferTooSmall)?;
        if bytes.as_ptr() as usize % tensor.layout.dtype.alignment() != 0 {
            return Err(Error::InvalidTensor);
        }
        Ok(TensorViewMut {
            bytes,
            layout: tensor.layout,
        })
    }
}

fn resolve(bytes: &[u8], tensor: SharedTensor) -> Result<&[u8], Error> {
    let start = tensor.buffer.offset as usize;
    let length =
        usize::try_from(tensor.layout.required_bytes()?).map_err(|_| Error::InvalidTensor)?;
    let end = start.checked_add(length).ok_or(Error::InvalidTensor)?;
    let view = bytes.get(start..end).ok_or(Error::BufferTooSmall)?;
    if view.as_ptr() as usize % tensor.layout.dtype.alignment() != 0 {
        return Err(Error::InvalidTensor);
    }
    Ok(view)
}
