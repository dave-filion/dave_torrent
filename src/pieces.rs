use lava_torrent::torrent::v1::Torrent;
use crate::BLOCK_SIZE;
use std::collections::VecDeque;
use crate::download::WorkChunk;

// handles dealing with blocks into pieces
pub struct PieceManager {
    pub num_pieces: usize, // total pieces in torrent
    pub piece_size: i64, // size of each piece
    pub block_size: u32, // max size of blocks, should be 2^14


}

impl PieceManager {
    pub fn new(num_pieces: usize, piece_size: i64, block_size: u32) -> Self {
        PieceManager{
            num_pieces,
            piece_size,
            block_size
        }
    }

    pub fn init_from_torrent(t: &Torrent) -> Self{
        PieceManager {
            num_pieces: t.pieces.len().clone(),
            piece_size: t.piece_length.clone(),
            block_size: BLOCK_SIZE,
        }
    }

    pub fn make_work_queue(&self) -> VecDeque<WorkChunk> {
        let mut queue = VecDeque::new();
        let num_pieces = self.num_pieces;
        let piece_size = self.piece_size as u32;
        let chunk_size = self.block_size as u32;
        println!("Making work queue with num_pieces:{}, piece_size:{}, chunk_size:{}...", num_pieces, piece_size, chunk_size);
        
        // seperate pieces into chunks
        for piece_index in 0..num_pieces {
            let mut i = 0;
            let mut block_id = 0;
            loop {
                if i == piece_size {
                    break
                } else if i + chunk_size > piece_size {
                    let len = piece_size - i;
                    queue.push_back(WorkChunk{
                        piece_index: piece_index as u32,
                        begin_index: i as u32,
                        length: len as u32,
                        block_id,
                    });
                    break
                } else {
                    queue.push_back(WorkChunk{
                        piece_index: piece_index as u32,
                        begin_index: i as u32,
                        length: chunk_size as u32,
                        block_id,
                    });

                    i += chunk_size;
                    block_id += 1;
                }
            }
        }
        queue
    }
}

