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

pub fn make_work_queue(num_pieces: usize, piece_size: u32, chunk_size: u32) -> VecDeque<WorkChunk> {
    println!("Making work queue with num_pieces:{}, piece_size:{}, chunk_size:{}...", num_pieces, piece_size, chunk_size);
    let mut queue = VecDeque::new();

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

#[cfg(test)]
mod test {
    use super::*;
    use crate::BLOCK_SIZE;

    #[test]
    fn test_make_work_queue() {
        let num_pieces = 4;
        let piece_size = 14;
        let chunk_size = 3;

        let mut q = make_work_queue(num_pieces, piece_size, chunk_size);
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

        let mut q = make_work_queue(num_pieces, piece_size, chunk_size);
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
        let mut q = make_work_queue(num_pieces, piece_size, chunk_size);
        println!("{} chunks", q.len());

        let first_work = q.pop_front().unwrap();
        println!("first = {:?}", first_work);

        let second = q.pop_front().unwrap();
        println!("second = {:?}", second);
    }
}