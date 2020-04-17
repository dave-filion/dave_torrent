use std::collections::VecDeque;

#[derive(Debug)]
pub struct WorkChunk {
    pub piece_index: u32,
    pub begin_index: u32,
    pub length: u32,
}

impl WorkChunk {
    pub fn new(p: u32, b: u32, l: u32) -> Self {
        WorkChunk {
            piece_index: p,
            begin_index: b,
            length: l
        }
    }
}

pub fn make_work_queue(num_pieces: u32, piece_size: u32, chunk_size: u32) -> VecDeque<WorkChunk> {
    let mut queue = VecDeque::new();

    // seperate pieces into chunks
    for piece_index in 0..num_pieces {
        let mut i = 0;
        loop {
            if i == piece_size {
                break
            } else if i + chunk_size > piece_size {
                let len = piece_size - i;
                queue.push_back(WorkChunk{
                    piece_index,
                    begin_index: i,
                    length: len,
                });
                break
            } else {
                queue.push_back(WorkChunk{
                    piece_index,
                    begin_index: i,
                    length: chunk_size,
                });

                i += chunk_size;
            }
        }

    }

    queue
}

#[cfg(test)]
mod test {
    use super::*;

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

        // real numbers
        let piece_size = 262144;
        let num_pieces = 1055;
        let chunk_size = 1028 * 10; // 10 kb
        let mut q = make_work_queue(num_pieces, piece_size, chunk_size);
        println!("{} chunks", q.len());

    }
}