#[derive(Debug)]
pub struct WorkChunk {
    pub piece_index: u32,
    pub begin_index: u32,
    pub length: u32,
    pub block_id: u32, // unique id per piece for each block
}

impl WorkChunk {
    pub fn new(p: u32, b: u32, l: u32, block_id: u32) -> Self {
        WorkChunk {
            piece_index: p,
            begin_index: b,
            length: l,
            block_id
        }
    }
}

#[derive(Debug)]
pub struct Block {
    pub data: Vec<u8>,
    pub piece_index: u32,
    pub offset: u32,
    pub block_id: u32,
}

