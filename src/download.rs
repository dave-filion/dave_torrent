use std::collections::VecDeque;

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


#[cfg(test)]
mod test {
    use super::*;
    use crate::BLOCK_SIZE;
    use crate::pieces::PieceManager;

    #[test]
    fn test_make_work_queue() {
        let num_pieces = 4;
        let piece_size = 14;
        let chunk_size = 3;

        let mut piece_manager = PieceManager::new(num_pieces, piece_size, chunk_size);
        let mut q = piece_manager.init_work_queue();

        // check piece_manager internals
        assert_eq!(piece_manager.expected_block_ids.len(), 4);
        assert_eq!(piece_manager.expected_block_ids.get(&3).unwrap().len(), 4);

        println!("piece_map: {:?}", piece_manager.piece_map);
        assert_eq!(piece_manager.piece_map.keys().len(), 4);

        println!("current block ids: {:?}", piece_manager.current_block_ids);
        assert_eq!(piece_manager.current_block_ids.keys().len(), 4);

        // try adding block
        let b = Block {
            data: vec![0, 1, 2, 3],
            piece_index: 0,
            offset: 0,
            block_id: 0,
        };
        piece_manager.add_block(b);
        // look at internals
        println!("piece_map: {:?}", piece_manager.piece_map);
        println!("current block ids: {:?}", piece_manager.current_block_ids);

        println!("{} chunks {:?}", q.len(), q);
        assert_eq!(q.len(), 20);

        let first = q.pop_front().unwrap();
        assert_eq!(first.piece_index, 0);
        assert_eq!(first.begin_index, 0);
        assert_eq!(first.length, 3);

        // test even number
        let num_pieces = 4;
        let piece_size = 9;
        let chunk_size = 3;

        let mut piece_manager = PieceManager::new(num_pieces, piece_size, chunk_size);
        let mut q = piece_manager.init_work_queue();
        println!("{} chunks {:?}", q.len(), q);
        assert_eq!(q.len(), 12);

        let first = q.pop_front().unwrap();
        assert_eq!(first.piece_index, 0);
        assert_eq!(first.begin_index, 0);
        assert_eq!(first.length, 3);
        assert_eq!(first.block_id, 0);

        // real numbers
        let piece_size = 262144;
        let num_pieces = 1055;
        let chunk_size = BLOCK_SIZE;
        let mut piece_manager = PieceManager::new(num_pieces, piece_size, chunk_size);
        let mut q = piece_manager.init_work_queue();
        println!("{} chunks", q.len());

        let first_work = q.pop_front().unwrap();
        println!("first = {:?}", first_work);

        let second = q.pop_front().unwrap();
        println!("second = {:?}", second);


    }
}