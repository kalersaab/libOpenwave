#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MdatBox {
    pub offset: u64,
    pub size: u64,
}

impl MdatBox {
    pub fn new(offset: u64, size: u64) -> Self {
        Self { offset, size }
    }
}
