use lava_torrent::torrent::v1::Torrent;
use crate::{BLOCK_SIZE, print_byte_array};
use std::collections::{VecDeque, HashMap, HashSet};
use crate::download::{WorkChunk, Block};
use failure::_core::str::from_utf8;

// handles dealing with blocks into pieces
pub struct PieceManager {
    pub num_pieces: usize, // total pieces in torrent
    pub piece_size: i64, // size of each piece
    pub block_size: u32, // max size of blocks, should be 2^14

    pub piece_map: HashMap<u32, HashMap<u32, Block>>,
    pub current_block_ids: HashMap<u32, HashSet<u32>>,
    pub expected_block_ids: HashMap<u32, HashSet<u32>>,

    pub piece_hashes: HashMap<u32, [u8; 20]>,

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
            piece_hashes: HashMap::new(),
        }
    }

    pub fn init_from_torrent(t: &Torrent) -> Self{
        // create piece hash map
        let mut piece_hashes : HashMap<u32, [u8; 20]>= HashMap::new();
        for (i, p) in t.pieces.iter().enumerate() {
            // print_byte_array(format!("piece: {}", i).as_str(), p);
            // convert from vec to byte array
            let mut ba = [0u8; 20];
            for j in 0..20 {
                let b = p.get(j).unwrap();
                ba[j] = b.clone();
            }
            piece_hashes.insert(i.clone() as u32, ba);
        }

        // println!("piece hashes: {:?}", piece_hashes);
        // let size = std::mem::size_of_val(&piece_hashes);
        // println!("piece hashes is {} bytes", size);

        PieceManager {
            num_pieces: t.pieces.len().clone(),
            piece_size: t.piece_length.clone(),
            block_size: BLOCK_SIZE,
            piece_map: HashMap::new(),
            current_block_ids: HashMap::new(),
            expected_block_ids: HashMap::new(),
            piece_hashes,
        }
    }

    pub fn add_block(&mut self, block: Block) {
        println!("Adding block: {}:{}", block.piece_index, block.block_id);

        // add to block id set
        self.current_block_ids.get_mut(&block.piece_index)
            .expect("Piece missing from block ids").insert(block.block_id);

        // store in piece_map
        self.piece_map.get_mut(&block.piece_index)
            .expect("Piece missing from piece map")
            .insert(block.block_id, block);

    }

    pub fn init_work_queue(&mut self) -> VecDeque<WorkChunk> {
        let mut queue = VecDeque::new();
        let num_pieces = self.num_pieces;
        let piece_size = self.piece_size as u32;
        let chunk_size = self.block_size as u32;

        // initialize internal data structs
        let mut piece_map: HashMap<u32, HashMap<u32, Block>>= HashMap::new();
        let mut current_block_ids : HashMap<u32, HashSet<u32>>= HashMap::new();
        let mut expected_block_ids : HashMap<u32, HashSet<u32>>= HashMap::new();

        println!("Making work queue with num_pieces:{}, piece_size:{}, chunk_size:{}...", num_pieces, piece_size, chunk_size);

        // seperate pieces into chunks
        for piece_index in 0..num_pieces {
            let mut i = 0;
            let mut block_id = 0;

            // init empty map for piece map
            piece_map.insert(piece_index as u32, HashMap::new());
            current_block_ids.insert(piece_index as u32, HashSet::new());

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
            expected_block_ids.insert(piece_index as u32, expected_block_id_set);

        }

        self.piece_map = piece_map;
        self.current_block_ids = current_block_ids;
        self.expected_block_ids = expected_block_ids;

        queue
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_init_from_torrent() {
        let filepath = "big-buck-bunny.torrent";
        let torrent = Torrent::read_from_file(filepath).unwrap();

        let piece_man = PieceManager::init_from_torrent(&torrent);

        // verify piece hash is expected
        let piece_hash : [u8; 20] = [0x84, 0x88, 0x1D, 0x13, 0x2F, 0xB4, 0x41, 0x89, 0x1B, 0xD8, 0x7F, 0xD7, 0x6A, 0xA2, 0x28, 0x33, 0x49, 0x7F, 0x2C, 0xFA];
        let other_hash = piece_man.piece_hashes.get(&1046).unwrap();

        assert_eq!(piece_hash, *other_hash);
    }
}
