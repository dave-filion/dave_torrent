use lava_torrent::torrent::v1::Torrent;
use crate::BLOCK_SIZE;
use std::collections::{VecDeque, HashMap, HashSet};
use crate::download::{WorkChunk, Block};

// handles dealing with blocks into pieces
pub struct PieceManager {
    pub num_pieces: usize, // total pieces in torrent
    pub piece_size: i64, // size of each piece
    pub block_size: u32, // max size of blocks, should be 2^14

    pub piece_map: HashMap<usize, HashMap<u32, Block>>,
    pub current_block_ids: HashMap<usize, HashSet<u32>>,
    pub expected_block_ids: HashMap<usize, HashSet<u32>>,


}

impl PieceManager {
    // dont use this, use init from torrent instead
    pub fn new(num_pieces: usize, piece_size: i64, block_size: u32) -> Self {
        PieceManager{
            num_pieces,
            piece_size,
            block_size,
            piece_map: HashMap::new(),
            current_block_ids: HashMap::new(),
            expected_block_ids: HashMap::new(),
        }
    }

    pub fn init_from_torrent(t: &Torrent) -> Self{
        PieceManager {
            num_pieces: t.pieces.len().clone(),
            piece_size: t.piece_length.clone(),
            block_size: BLOCK_SIZE,
            piece_map: HashMap::new(),
            current_block_ids: HashMap::new(),
            expected_block_ids: HashMap::new()
        }
    }

    pub fn add_block(&mut self, block: Block) {
        println!("Adding block: {}:{}", block.piece_index, block.block_id);

    }

    pub fn init_work_queue(&mut self) -> VecDeque<WorkChunk> {
        let mut queue = VecDeque::new();
        let num_pieces = self.num_pieces;
        let piece_size = self.piece_size as u32;
        let chunk_size = self.block_size as u32;

        // initialize internal data structs
        let mut piece_map: HashMap<usize, HashMap<u32, Block>>= HashMap::new();
        let mut current_block_ids : HashMap<usize, HashSet<u32>>= HashMap::new();
        let mut expected_block_ids : HashMap<usize, HashSet<u32>>= HashMap::new();

        println!("Making work queue with num_pieces:{}, piece_size:{}, chunk_size:{}...", num_pieces, piece_size, chunk_size);

        // seperate pieces into chunks
        for piece_index in 0..num_pieces {
            let mut i = 0;
            let mut block_id = 0;

            // init empty map for piece map
            piece_map.insert(piece_index, HashMap::new());
            current_block_ids.insert(piece_index, HashSet::new());

            let mut expected_block_id_set = HashSet::new();

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

                    // add block id entries to set
                    expected_block_id_set.insert(block_id.clone());

                    i += chunk_size;
                    block_id += 1;
                }
            }

            // add to map
            expected_block_ids.insert(piece_index, expected_block_id_set);

        }

        self.piece_map = piece_map;
        self.current_block_ids = current_block_ids;
        self.expected_block_ids = expected_block_ids;

        queue
    }
}

