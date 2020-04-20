use lava_torrent::torrent::v1::Torrent;
use crate::{BLOCK_SIZE, print_byte_array};
use std::collections::{VecDeque, HashMap, HashSet};
use crate::download::{WorkChunk, Block};
use failure::_core::str::from_utf8;
use bytebuffer::ByteBuffer;

#[derive(Debug)]
pub struct PieceData {
    pub id: u32,
    pub data: Vec<u8>,
}

// handles dealing with blocks into pieces
#[derive(Debug)]
pub struct PieceManager {
    pub num_pieces: usize, // total pieces in torrent
    pub piece_size: i64, // size of each piece
    pub block_size: u32, // max size of blocks, should be 2^14

    pub piece_map: HashMap<u32, HashMap<u32, Block>>,
    pub current_block_ids: HashMap<u32, HashSet<u32>>,
    pub expected_block_ids: HashMap<u32, HashSet<u32>>,

    pub piece_hashes: HashMap<u32, [u8; 20]>,
    pub finished_pieces: HashMap<u32, PieceData>,

}

fn init_finished_pieces(n: usize) -> HashMap<u32, PieceData> {
    // init finished pieces map with optionals
    let mut fp = HashMap::new();
    fp
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
            finished_pieces: init_finished_pieces(num_pieces),
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
        let np = t.pieces.len().clone();

        PieceManager {
            num_pieces: np,
            piece_size: t.piece_length.clone(),
            block_size: BLOCK_SIZE,
            piece_map: HashMap::new(),
            current_block_ids: HashMap::new(),
            expected_block_ids: HashMap::new(),
            piece_hashes,
            finished_pieces: init_finished_pieces(np),
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

    // assemble a piece from blocks and check its hash
    pub fn assemble_piece(&mut self, piece_id: u32) -> PieceData {
        println!("Trying to assemble piece: {}",piece_id);

        let mut data = ByteBuffer::new();

        let blocks = self.piece_map.get(&piece_id).expect("Couldnt get blocks for piece id");
        let block_ids = self.expected_block_ids.get(&piece_id).expect("Couldnt get expected block ids for piece");

        // iterate thru block ids in order and add bytes to data buffer
        let mut block_ids: Vec<u32> = block_ids.iter().map(|id| id.clone()).collect();
        block_ids.sort();

        for b_id in &block_ids {
            println!("getting block data for bid: {}", b_id);
            let block = blocks.get(b_id).unwrap();
            data.write_bytes(&block.data);
            println!("wrote block {} data to buffer, new size: {}", b_id, data.len());
        }

        PieceData {
            id: piece_id,
            data: data.to_bytes()
        }
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

    #[test]
    fn test_assemble_piece() {
        let num_pieces = 3;
        let piece_size = 10;
        let block_size = 5;
        let mut pm = PieceManager::new(num_pieces, piece_size, block_size);
        let wq = pm.init_work_queue();

        println!("pm => {:?}", pm);
        let block1 = Block{
            data: vec![0,1,2,3,4],
            piece_index: 0,
            offset: 0,
            block_id: 0
        };
        pm.add_block(block1);

        let block2 = Block{
            data: vec![5,6,7,8,9],
            piece_index: 0,
            offset: 5,
            block_id: 1
        };
        pm.add_block(block2);

        // try assembling piece 0
        let piece_data = pm.assemble_piece(0);
        println!("piece data: {:?}", piece_data);
        assert_eq!(piece_data.id, 0);
        assert_eq!(piece_data.data.len(), 10);
    }
}
