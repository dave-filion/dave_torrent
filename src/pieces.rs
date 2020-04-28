use lava_torrent::torrent::v1::Torrent;
use crate::{BLOCK_SIZE, print_byte_array};
use std::collections::{VecDeque, HashMap, HashSet};
use crate::download::{WorkChunk, Block};
use failure::_core::str::from_utf8;
use bytebuffer::ByteBuffer;
use std::path::{Path, PathBuf};
use std::fs::File;
use std::io::prelude::*;
use std::fs;
use sha1::{Sha1, Digest};
use std::time::{Duration, SystemTime};
use failure::Error;
use std::ffi::OsStr;
use crate::logging::debug;

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
    pub output_dir: String,

    pub last_ts: SystemTime,

}

pub fn sha_data(data: &Vec<u8>) -> Vec<u8> {
    let mut hasher = Sha1::new();
    hasher.input(data);
    let result = hasher.result();
    result.to_vec()
}

pub fn make_piece_data_filename(piece_id: u32) -> String {
    return format!("{}.dave", piece_id);
}

pub fn read_piece_from_file(dir: &str, piece_id: u32) -> Result<PieceData, Error> {
    let path = format!("{}/{}", dir, make_piece_data_filename(piece_id));
    debug(format!("Reading piece from file: {}", path));

    let mut f = File::open(path)?;

    // read all data
    let mut data = Vec::new();
    let bytes_read = f.read_to_end(&mut data)?;
    debug(format!("Read {} bytes from file", bytes_read));

    Ok(PieceData {
        id: piece_id,
        data,
    })
}

// outputs piece data to file
pub fn write_piece_to_file(output_dir: &str, piece: PieceData) {
    // make dave files (data files)
    let p = format!("{}/{}", output_dir, make_piece_data_filename(piece.id));
    let path = Path::new(p.as_str());
    debug(format!("writing piece {} to filename: {:?}", piece.id, path));

    // create dir if not present
    fs::create_dir_all(output_dir).expect("Error creating dir");

    let mut file = match File::create(path) {
        Ok(file) => file,
        Err(e) => panic!("Couldnt create file: {:?}: {:?}", path, e)
    };

    match file.write_all(piece.data.as_slice()) {
        Ok(_) => debug(format!("Wrote piece {} to file successfully", piece.id)),
        Err(e) => panic!("Error writing piece to file: {:?}", e),
    }

}

fn init_finished_pieces(n: usize) -> HashMap<u32, PieceData> {
    // init finished pieces map with optionals
    let mut fp = HashMap::new();
    fp
}

pub fn get_ext_from_filename(filename: &str) -> Option<&str> {
    Path::new(filename)
        .extension()
        .and_then(OsStr::to_str)
}

impl PieceManager {
    // dont use this, use init from torrent instead
    pub fn new(num_pieces: usize, piece_size: i64, block_size: u32, output_dir: String) -> Self {
        let now = SystemTime::now();

        PieceManager{
            num_pieces,
            piece_size,
            block_size,
            piece_map: HashMap::new(),
            current_block_ids: HashMap::new(),
            expected_block_ids: HashMap::new(),
            piece_hashes: HashMap::new(),
            finished_pieces: init_finished_pieces(num_pieces),
            output_dir,
            last_ts: now,
        }
    }

    pub fn init_from_torrent(t: &Torrent, output_dir: String) -> Self{
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

        // debug(format!("piece hashes: {:?}", piece_hashes);
        // let size = std::mem::size_of_val(&piece_hashes);
        // debug(format!("piece hashes is {} bytes", size);
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
            output_dir,
            last_ts: SystemTime::now(),
        }
    }

    pub fn add_block(&mut self, block: Block) {
        debug(format!("Adding block: {}:{}", block.piece_index, block.block_id));

        // print time to download block
        let block_size = block.data.len();
        match self.last_ts.elapsed() {
            Ok(elapsed) => {
                let millis = elapsed.as_millis();
                debug(format!("{} bytes in {}ms", block_size, millis));
                // debug(format!("{} bytes per ms", block_size / millis as usize);
            },
            Err(e) => {
                debug(format!("error getting system time: {:?}", e));
            }
        }
        self.last_ts = SystemTime::now();

        let piece_index = block.piece_index.clone();

        // add to block id set
        self.current_block_ids.get_mut(&block.piece_index)
            .expect("Piece missing from block ids").insert(block.block_id);

        // store in piece_map
        self.piece_map.get_mut(&block.piece_index)
            .expect("Piece missing from piece map")
            .insert(block.block_id, block);


        if self.expected_block_ids.get(&piece_index) == self.current_block_ids.get(&piece_index) {
            debug(format!("We have all blocks for piece {}, assembling...", piece_index));
            let piece_data = self.assemble_piece(piece_index);

            match self.piece_hashes.get(&piece_index) {
                Some(verified_hash) => {
                    //TODO: actually check that hashes match!
                    debug(format!("Checking verified hash for piece {}:", piece_index));
                    print_byte_array("->", verified_hash);
                    let hash = sha_data(&piece_data.data);
                    print_byte_array("->", &hash);
                },
                None => {
                    debug(format!("No hash found for piece {} in torrent! Assuming its contents are correct", piece_index));
                }
            }

            // write to file
            write_piece_to_file(self.output_dir.as_str(), piece_data);

            // remove block data for piece and clean up
            self.remove_piece_data(piece_index);
        }
    }

    // remove all data in memory for piece (block data)
    pub fn remove_piece_data(&mut self, piece_index: u32) {
        debug(format!("removing all piece data for piece: {}", piece_index));

        match self.piece_map.remove(&piece_index) {
            Some(result) => {
                debug(format!("removed and dropping piece data for id : {}, total size: {}", piece_index, std::mem::size_of_val(&result)));

                // drop, although should be dropped automatically
                std::mem::drop(result);
            },
            None => {
                debug(format!("Couldnt remove piece index from piece map! {}", piece_index));
            }
        }
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

        debug(format!("Making work queue with num_pieces:{}, piece_size:{}, chunk_size:{}...", num_pieces, piece_size, chunk_size));

        // check if there are existing pieces, if so, don't add them back in
        debug(format!("checking if pieces downloaded already at: {}", self.output_dir));
        let mut completed_pieces = HashSet::new();
        if Path::new(self.output_dir.as_str()).exists() {
            debug(format!("found {:?} exists already, checking for pieces...", self.output_dir));
            // load all files of type *.dave in output
            let paths = fs::read_dir(self.output_dir.as_str()).unwrap();
            for p in  paths {
                let file = p.unwrap();
                let filename = file.file_name();
                let ext = get_ext_from_filename(&filename.to_str().unwrap());

                // need to make sure only .dave files are checked
                if ext.is_some() && ext.unwrap() != "dave" {
                    debug(format!("skipping file with ext: {:?}", ext.unwrap()));
                    continue
                }
                let path = file.path();
                let stem = path.file_stem().unwrap().to_str().unwrap();
                let parsed: usize = stem.parse().unwrap(); // TODO error checking
                debug(format!("found piece = {:?}", parsed));
                completed_pieces.insert(parsed);
            }
            debug(format!("Found {} already completed pieces", completed_pieces.len()));
        } else {
            debug(format!("No existing output dir found... starting from scratch"));
        }

        // seperate pieces into chunks
        for piece_index in 0..num_pieces {

            if completed_pieces.contains(&piece_index) {
                debug(format!("piece {} was already completed! skipping", piece_index));
                continue;
            }

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
                    expected_block_id_set.insert(block_id.clone());

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
        debug(format!("Assembling piece: {}",piece_id));

        let mut data = ByteBuffer::new();

        let blocks = self.piece_map.get(&piece_id).expect("Couldnt get blocks for piece id");
        let block_ids = self.expected_block_ids.get(&piece_id).expect("Couldnt get expected block ids for piece");

        // iterate thru block ids in order and add bytes to data buffer
        let mut block_ids: Vec<u32> = block_ids.iter().map(|id| id.clone()).collect();
        block_ids.sort();

        for b_id in &block_ids {
            let block = blocks.get(b_id).unwrap();
            data.write_bytes(&block.data);
        }

        PieceData {
            id: piece_id,
            data: data.to_bytes()
        }
    }
}

pub fn assemble_files_from_pieces(output_dir: &str, file_list: Vec<(PathBuf, i64)>) {
    let mut current_piece = 0;
    let mut buf = ByteBuffer::new();
    for output_file in file_list {
        let (filename, needed_length) = output_file;
        let needed_length = needed_length as usize;
        debug(format!("Trying to write {} bytes to file: {:?}", needed_length, filename));

        // read in enough bytes from pieces to write file
        loop {
            if buf.len() < needed_length {
                debug(format!("buf ({}b) not long enough (need {}b), reading next piece: {}", buf.len(), needed_length, current_piece));

                // read in piece
                match read_piece_from_file(output_dir, current_piece) {
                    Ok(piece) => {

                        // write piece data to buf
                        buf.write_all(piece.data.as_slice());

                        current_piece += 1;
                    },
                    Err(_e) => {
                        debug(format!("Out of files to read!"));
                        return;
                    }
                }

            } else {
                debug(format!("buf ({}b) is long enough for file: ({}b), writing file", buf.len(), needed_length));

                // write buf[0..needed_len] to file
                let file_data = buf.to_bytes();

                // get file slice
                let file_slice = &file_data[0..needed_length];
                // print_byte_array(format!("{:?}", filename).as_str(), file_slice);
                // wrrite slice to file
                let new_file_path = Path::new(output_dir).join(filename);
                debug(format!("new file path: {:?}", new_file_path));
                let mut new_file = File::create(&new_file_path).expect("Couldnt open new file to write to");
                new_file.write_all(file_slice);
                debug(format!("wrote {} bytes to file: {:?}", file_slice.len(), new_file_path));

                // update buf such that
                let remaining_in_buf = &file_data[needed_length..file_data.len()];
                debug(format!("{} bytes remaining in buf, moving to new buffer", remaining_in_buf.len()));

                let mut buf = ByteBuffer::new();
                buf.write_bytes(remaining_in_buf);
                break;
            }
        }
    }
}


#[cfg(test)]
mod test {
    use super::*;
    use crate::print_torrent_info;
    use rand::thread_rng;
    use rand::Rng;
    use std::fs::{self, ReadDir};
    use std::thread;
    use std::time::Duration;
    use crate::app::{make_output_dir, make_specific_output_dir};

    fn clear_test_dir(dir: &str) {
        if let Ok(d) = fs::read_dir(dir) {
            for f in d {
                if let Ok(f) = f {
                    let path = f.path();
                    println!("removing file: {:?}", path);
                    fs::remove_file(path);
                }
            }
        }

    }

    #[test]
    fn test_init_from_torrent() {
        let filepath = "big-buck-bunny.torrent";
        let torrent = Torrent::read_from_file(filepath).unwrap();

        // how do files relate to pieces?
        print_torrent_info(&torrent);

        let piece_man = PieceManager::init_from_torrent(&torrent, "test/output".to_string());

        // verify piece hash is expected
        let piece_hash : [u8; 20] = [0x84, 0x88, 0x1D, 0x13, 0x2F, 0xB4, 0x41, 0x89, 0x1B, 0xD8, 0x7F, 0xD7, 0x6A, 0xA2, 0x28, 0x33, 0x49, 0x7F, 0x2C, 0xFA];
        let other_hash = piece_man.piece_hashes.get(&1046).unwrap();

        assert_eq!(piece_hash, *other_hash);
    }

    #[test]
    fn test_write_piece_to_file() {
        clear_test_dir("/test/output3");

        let mut d = Vec::new();
        // write 1000 bytes
        for i in 0..100 {
            d.push(i);
        }

        let piece = PieceData{
            id: 1,
            data: d,
        };
        write_piece_to_file("test/output3", piece);
    }
    //
    // #[test]
    // fn test_process_piece() {
    //     clear_test_dir("test/output4");
    //
    //     let b1 = Block{
    //         data: vec![0x01, 0x02, 0x03, 0x04, 0x05],
    //         piece_index: 0,
    //         offset: 0,
    //         block_id: 0
    //     };
    //     let b2 = Block{
    //         data: vec![0x06, 0x07, 0x08, 0x09, 0x10],
    //         piece_index: 0,
    //         offset: 5,
    //         block_id: 1
    //     };
    //     let b3 = Block{
    //         data: vec![0x11, 0x12, 0x13, 0x14, 0x15],
    //         piece_index: 0,
    //         offset: 10,
    //         block_id: 2
    //     };
    //
    //     let mut piece_man = PieceManager::new(1, 15, 5, "test/output4".to_string());
    //     let mut workqueue = piece_man.init_work_queue();
    //     println!("wq = {:?}", workqueue);
    //
    //     piece_man.add_block(b1);
    //     piece_man.add_block(b2);
    //     piece_man.add_block(b3);
    //
    //     // wait a sec
    //     thread::sleep(Duration::from_millis(500));
    //
    //     // file should have been created
    //     let mut data_file = File::open("test/output4/0.dave").expect("Should be there");
    //     let mut buf = Vec::new();
    //     let bytes_read = data_file.read_to_end(&mut buf).unwrap();
    //     println!("Read {} bytes from data file", bytes_read);
    //     assert_eq!(bytes_read, 15);
    //
    //     print_byte_array("data_file", &buf);
    //     assert_eq!(buf, vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15]);
    //
    //     // verify piece data is no longer in piece manager
    //     assert!(piece_man.piece_map.get(&0).is_none());
    // }

    #[test]
    fn test_process_piece_with_larger_file() {
        clear_test_dir("test/output5");

        // use rand::seq::SliceRandom;

        let num_blocks = 20;
        let block_size = 100;

        println!("Making {} blocks of size: {}", num_blocks, block_size);
        let mut data_buf = ByteBuffer::new();
        let mut blocks = Vec::new();

        // make random blocks
        let mut thread_rng = thread_rng();
        for i in 0..num_blocks {
            let data = thread_rng.gen::<[u8; 8]>();
            data_buf.write_bytes(&data);

            let block = Block{
                data: data.to_vec(),
                piece_index: 0,
                offset: i * block_size,
                block_id: i
            };
            blocks.push(block);
        }

        println!("done making blocks, shuffling and assembling");
        // blocks.shuffle(&mut thread_rng);


        // init piece manager
        let mut pm = PieceManager::new(1, (num_blocks * block_size) as i64, block_size, "test/output5".to_string());
        let wq = pm.init_work_queue();
        println!("Initialized work queue with {} entries", wq.len());
        // verify hash and write to pieceman
        let data = data_buf.to_bytes();
        let verified_hash = sha_data(&data);
        println!("data hash is {:?}", verified_hash);
        // pm.piece_hashes.insert(0, verified_hash);

        // add blocks to piece man
        for block in blocks {
            println!("adding block {}", block.block_id);
            pm.add_block(block)
        }

        // verify piece written to file
        let piece_data = read_piece_from_file("test/output5", 0);

    }

    #[test]
    fn test_read_piece_from_file() {
        // real torrent piece
        let piece = read_piece_from_file("test/sha-test", 0).unwrap();
        let my_sha = sha_data(&piece.data);

        // get piece data from torrent
        let filepath = "big-buck-bunny.torrent";
        let torrent = Torrent::read_from_file(filepath).unwrap();

        let the_sha =torrent.pieces.get(0)
            .expect("get 1st piece")
            .clone();

        println!("These should match:");
        print_byte_array("->", &my_sha);
        print_byte_array("->", &the_sha);
        assert_eq!(the_sha, my_sha);
    }

    #[test]
    fn test_resume_from_pieces() {
        let filepath = "big-buck-bunny.torrent";
        let torrent = Torrent::read_from_file(filepath).unwrap();
        let output_dir = make_specific_output_dir("test/resume-pieces", filepath);
        let mut piece_man = PieceManager::init_from_torrent(&torrent, output_dir);
        let wq = piece_man.init_work_queue();

        // verify piece manager doesnt have entries for piece 0 or 1
        assert_eq!(wq.len(), 16848);
        assert!(piece_man.piece_map.get(&0).is_none());
        assert!(piece_man.piece_map.get(&1).is_none());
    }

    #[test]
    fn test_build_files_from_pieces() {
        let output_dir = "test/assembly";

        // each piece is 5 bytes
        let p1_data = vec![0x01, 0x01, 0x01, 0x01, 0x01];
        let p2_data = vec![0x01, 0x01, 0x01, 0x02, 0x02];
        let p3_data = vec![0x02, 0x02, 0x02, 0x03, 0x03];

        // write pieces to files
        write_piece_to_file(output_dir, PieceData{
            id: 0,
            data: p1_data,
        });
        write_piece_to_file(output_dir, PieceData{
            id: 1,
            data: p2_data,
        });
        write_piece_to_file(output_dir, PieceData{
            id: 2,
            data: p3_data,
        });


        let file1 = (PathBuf::from("f1.txt"), 8);
        let file2 = (PathBuf::from("f2.txt"), 5);
        let file3 = (PathBuf::from("f3.txt"), 2);

        let mut file_list = Vec::new();
        file_list.push(file1);
        file_list.push(file2);
        file_list.push(file3);

        assemble_files_from_pieces(output_dir, file_list);

        // verify files are correct
        let f1 = File::open(format!("{}/{}", output_dir, "f1.txt")).unwrap();
        assert_eq!(f1.metadata().unwrap().len(), 8);

        let f2 = File::open(format!("{}/{}", output_dir, "f2.txt")).unwrap();
        assert_eq!(f2.metadata().unwrap().len(), 5);

        let f3 = File::open(format!("{}/{}", output_dir, "f3.txt")).unwrap();
        assert_eq!(f3.metadata().unwrap().len(), 2);
    }

}

