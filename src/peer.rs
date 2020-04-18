use rand::{AsByteSliceMut, Rng};
use std::io::{Cursor, Read, Write};
use std::thread;

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::str::from_utf8;
use std::time::Duration;
use std::sync::mpsc::{channel, Sender};
use failure::Error;
use failure::err_msg;

use crate::download::{WorkChunk, Block};
use crate::*;
use std::collections::VecDeque;

// higher level function, tries connecting to peer, handshake, and start downloading data
pub fn attempt_peer_download(
    ip: IpAddr,
    port: u16,
    info_hash_array: &[u8; 20],
    peer_id: &[u8; 20],
    work_queue: &mut VecDeque<WorkChunk>,
    processing_chan: Sender<Block>,
) -> Result<(), Error> {
    println!("\n---------------------\n");
    println!("Connecting to peer: {:?}:{}", ip, port);
    let peer_result = Peer::new(
        ip.clone(),
        port.clone(),
        info_hash_array.clone(),
        peer_id.clone(),
    );

    if let Err(e) = peer_result {
        return Err(err_msg(e));
    }

    let mut peer = peer_result.unwrap();
    let handshake_result = peer.perform_handshake();
    if let Err(e) = handshake_result {
        return Result::Err(err_msg(e));
    }

    // actually not garbage, is bitfield
    // TODO recv bitfield
    peer.recv_garbage();
    peer.send_interested();

    if peer.recv_unchoke() {
        // start pulling work off work queue
        println!("Peer unchoked...Starting to pull work off work queue");
        loop {
            match work_queue.pop_front() {
                Some(next_chunk) => {
                    print!("Downloading block {}:{}...", next_chunk.piece_index, next_chunk.block_id);
                    match peer.fetch_block_data(&next_chunk) {
                        Ok(data) => {
                            println!("got it!");
                            // print_byte_array("piece data", &piece_data);
                            let block = Block {
                                data,
                                piece_index: next_chunk.piece_index,
                                offset: next_chunk.begin_index,
                                block_id: next_chunk.block_id,
                            };

                            if let Err(e) = processing_chan.send(block) {
                                println!("error sending block to processing thread! Putting chunk back on work queue and breaking");
                                work_queue.push_back(next_chunk);
                                break;
                            }
                        },
                        Err(e) => {
                            println!("Error getting piece data, putting chunk back on queue, and breaking");
                            work_queue.push_back(next_chunk);
                            break;
                        }
                    }
                }
                None => {
                    println!("No more work on queue! breaking out of loop");
                    break;
                }
            }
        }
        // at this point, the connection ended successfully
        return Result::Ok(());
    } else {
        return Result::Err(err_msg("Couldnt unchoke peer"));
    }
}

// tcp connection to peer
pub fn connect_to_peer(ip: IpAddr, port: u16) -> Result<TcpStream, Error> {
    let sock_addr = SocketAddr::new(ip, port);
    // 5 sec connection timeout
    if let Ok(stream) = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5)) {
        // set stream to blocking
        stream
            .set_nonblocking(false)
            .expect("cant set stream to blocking");
        Result::Ok(stream)
    } else {
        Result::Err(err_msg("Can't connect to peer"))
    }
}

// makes a tcp handshake packet
// handshake: <pstrlen><pstr><reserved><info_hash><peer_id>
//
// pstrlen: string length of <pstr>, as a single raw byte
// pstr: string identifier of the protocol
// reserved: eight (8) reserved bytes. All current implementations use all zeroes.
// peer_id: 20-byte string used as a unique ID for the client.
//
// In version 1.0 of the BitTorrent protocol, pstrlen = 19, and pstr = "BitTorrent protocol".
pub fn make_handshake(peer_id: &[u8; 20], info_hash: &[u8; 20]) -> Vec<u8> {
    let mut buf = ByteBuffer::new();

    // pstrlen
    buf.write_bytes(&[19u8; 1]);

    // pstr
    let prot_string = "BitTorrent protocol";
    let prot_bytes = prot_string.as_bytes();
    buf.write_bytes(prot_bytes);

    // reserved bytes
    buf.write_bytes(&[0u8; 8]);

    // info hash
    buf.write_bytes(info_hash);

    // peer id
    buf.write_bytes(peer_id);

    buf.to_bytes()
}

#[derive(Debug)]
pub struct HandshakeResponse {
    pub protocol: String,
    pub info_hash: Vec<u8>, //20 bytes
    pub peer_id: Vec<u8>,   // 20 bytes
}

pub fn parse_handshake_response(buf: &Vec<u8>) -> HandshakeResponse {
    // 1 byte = 19
    let strlen = buf.get(0).unwrap();

    // get pstr
    let (pstr, new_i) = get_n_bytes_at(&buf, 1, strlen.clone() as usize);
    // verify protocol is correct
    let prot = from_utf8(&pstr).expect("protocol not parsable to str");

    // get info hash
    let (info_hash, new_i) = get_n_bytes_at(&buf, (1 + strlen.clone() + 8) as usize, 20);

    // get peer_id
    let (peer_id, _) = get_n_bytes_at(&buf, new_i + 1, 20);

    HandshakeResponse {
        protocol: prot.to_string(),
        info_hash,
        peer_id,
    }
}

// keep-alive: <len=0000>
// choke: <len=0001><id=0>
// unchoke: <len=0001><id=1>
// interested: <len=0001><id=2>
// not interested: <len=0001><id=3>
// have: <len=0005><id=4><piece index>
// bitfield: <len=0001+X><id=5><bitfield>
// request: <len=0013><id=6><index><begin><length>
// piece: <len=0009+X><id=7><index><begin><block>
// cancel: <len=0013><id=8><index><begin><length>
// port: <len=0003><id=9><listen-port>
#[derive(Debug)]
pub enum PeerMessage {
    KeepAlive,
    Choke(u32),                  // id = 0
    Unchoke(u32),                // id = 1
    Interested(u32),             // id = 2
    NotInterested(u32),          // id = 3
    Have(u32, u32),              // id = 4, piece_index=4bytes
    Bitfield(u32, Vec<u8>),      // id = 5, bitfield
    Request(u32, u32, u32, u32), // id = 6, index=4, begin=4, length=4
    Piece(u32, u32, u32, Option<Vec<u8>>),         // id = 6, index=4, offset/begin=4, vector of piece data
    Cancel(u32),
    Port(u32),
}

pub fn make_choke_msg() -> Vec<u8> {
    let mut buf = ByteBuffer::new();
    // length
    buf.write_u32(1);
    // id
    buf.write_u8(0);

    buf.to_bytes()
}

pub fn make_unchoke_msg() -> Vec<u8> {
    let mut buf = ByteBuffer::new();
    // length
    buf.write_u32(1);
    // id
    buf.write_u8(1);

    buf.to_bytes()
}

pub fn make_interested_msg() -> Vec<u8> {
    let mut buf = ByteBuffer::new();
    // length
    buf.write_u32(1);
    // id
    buf.write_u8(2);

    buf.to_bytes()
}

pub fn make_uninterested_msg() -> Vec<u8> {
    let mut buf = ByteBuffer::new();
    // length
    buf.write_u32(1);
    // id
    buf.write_u8(3);

    buf.to_bytes()
}

pub fn make_have_msg(piece_index: u32) -> Vec<u8> {
    let mut buf = ByteBuffer::new();
    // length
    buf.write_u32(5);
    // id
    buf.write_u8(4);
    // piece index
    buf.write_u32(piece_index);

    buf.to_bytes()
}

pub fn make_request_msg(piece_index: u32, begin: u32, len: u32) -> Vec<u8> {
    let mut buf = ByteBuffer::new();
    // msg len
    buf.write_u32(13);

    // id
    buf.write_u8(6);

    // piece
    buf.write_u32(piece_index);

    // begin
    buf.write_u32(begin);

    // len
    buf.write_u32(len);

    buf.to_bytes()
}

pub struct Peer {
    choked: bool,
    stream: TcpStream,
    ip: IpAddr,
    port: u16,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

impl Peer {
    // Makes new peer and connects
    pub fn new(
        ip: IpAddr,
        port: u16,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
    ) -> Result<Self, Error> {
        let sock_addr = SocketAddr::new(ip, port);
        // 5 sec connection timeout
        if let Ok(stream) = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(2)) {
            // set stream to blocking
            stream
                .set_nonblocking(false)
                .expect("cant set stream to blocking");
            Result::Ok(Peer {
                choked: false,
                stream,
                ip,
                port,
                info_hash,
                peer_id,
            })
        } else {
            Result::Err(err_msg("Cant connect to peer"))
        }
    }

    // for some reason after handshake theres a bunch of nonesense sent, read it out of queue
    pub fn recv_garbage(&mut self) {
        let mut buf = [0; 512];
        let read_result = self.stream.read(&mut buf);
        match read_result {
            Ok(bytes_read) => {
                print_byte_array_len("bitfield", &buf, bytes_read);
            }
            Err(e) => {}
        }
    }

    pub fn maybe_revc_bitfield(&mut self) {
        println!("reading for 5 sec to see if bitfield recv");
        self.stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        // read 4 byte size
        let mut buf = [0; 4];
        let read_result = self.stream.read(&mut buf);
        match read_result {
            Ok(bytes_read) => {
                println!("read bytes: {}", bytes_read);
                let msg_size = BigEndian::read_u32(&buf);

                // for some reason, getting huge numbers here, just skip
                if msg_size > 512 {
                    println!("Got message size of {}, way too big, skipping", msg_size);
                    return;
                }

                println!("msg size is {} bytes, reading...", msg_size);

                let mut buf = Vec::with_capacity(msg_size as usize);
                let read_result = self.stream.read(&mut buf);

                match read_result {
                    Ok(bytes_read) => {
                        println!("read msg bytes: {}", bytes_read);
                        print_byte_array("peer msg", &buf);
                        // return data here
                    }
                    Err(e) => {
                        println!("Error reading remaineder of message: {:?}", e);
                        // return none here
                    }
                }
            }
            Err(e) => {
                println!("Error reading message: {:?}", e);
                // return none here
            }
        }
    }

    pub fn send_interested(&mut self) {
        print!("Sending interested message...");
        let msg = make_interested_msg();
        self.stream.write_all(&msg).unwrap();
        println!("sent");
    }

    pub fn recv_unchoke(&mut self) -> bool {
        self.stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("Cant set read timeout");
        print!("Waiting for unchoke message...");
        let mut buf = [0; 512];
        let read_result = self.stream.read(&mut buf);
        match read_result {
            Ok(bytes) => {
                println!("received, unchoking peer");
                // TODO: check if msg is unchoke, if so, set unchoke
                parse_peer_msg(&buf);
                self.choked = false;
                true
            }
            Err(e) => {
                println!("error reading choke msg : {:?}", e);
                false
            }
        }
    }

    // sends request to peer for piece_index, with offset begin and length len
    // returns result with piece data, or error
    pub fn fetch_block_data(&mut self, work: &WorkChunk) -> Result<Vec<u8>, Error> {
        let block_id = work.block_id;
        let piece_id = work.piece_index;
        let offset = work.begin_index;
        let length = work.length;

        println!("Attempting to fetch Block: {}-{}", piece_id, block_id);

        // write request to stream
        let req = make_request_msg(piece_id, offset, length);
        self.stream.write_all(&req).expect("write all failed"); //TODO error handle correctly

        // set longer read timeout in case of slow connection
        self.stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        // since it may take multiple messages to send all data, need to concat to this buffer
        let mut all_data = Vec::new();

        // keep reading stream until all data for block has been received
        let mut i = 0;
        let mut total_msg_size: isize = 0; // is set on first read
        let mut total_bytes_read : isize = 0;
        loop {

            let mut read_buf = [0; 5000];
            let bytes_read = self.stream.read(&mut read_buf).expect("read filed"); // TODO error handle correctly

            if i == 0 {
                // slice only section of buf written to
                let sliced_buf = &read_buf[..bytes_read];
                if let Some(msg) = parse_peer_msg(sliced_buf) {
                    if let PeerMessage::Piece(msg_size, piece_id, offset, data) = msg {
                        println!("read #{}: {} bytes read, msg size: {}", i+1, bytes_read, msg_size);
                        total_msg_size = msg_size as isize;
                        total_bytes_read += bytes_read as isize;

                        if let Some(d) = data {
                            println!("size of data: {} bytes", d.len());
                            all_data.extend(d);
                        } else {
                            println!("msg had no piece data");
                        }

                        println!("{} bytes remaining", total_msg_size - total_bytes_read);
                        println!("current data size is {}", all_data.len());

                    } else {
                        return Err(err_msg("Peer message was not a piece msg as expected! returning error"));
                    }
                } else {
                    return Err(err_msg("Failed to parse peer message, returning error"));
                }
            } else {
                // second or more reads, append data to buffer
                println!("read #{}: {} bytes read", i+1, bytes_read);
                total_bytes_read += bytes_read as isize;
                let remaining_to_read = total_msg_size - total_bytes_read;
                println!("{} bytes remaining", remaining_to_read);

                // append data
                let sliced_buf = &read_buf[..bytes_read];
                all_data.extend_from_slice(sliced_buf);

                println!("current data size is: {}", all_data.len());

                if remaining_to_read <= 0 {
                    println!("0 bytes remaining to read, returning");
                    println!("final data size is: {}", all_data.len());
                    return Ok(all_data)
                } else {
                    println!("still more bytes to go! will read again in a sec");
                }
            }
            // wait a sec then read again
            thread::sleep(Duration::from_secs(1));
            i+=1;
        }
    }

    pub fn perform_handshake(&mut self) -> Result<(), Error> {
        print!("performing handshake...");
        let handshake = make_handshake(&self.peer_id, &self.info_hash);

        // write handshake to stream
        let write_result = self.stream.write_all(&handshake);
        if let Ok(bytes_wrote) = write_result {
            // set read timeout
            self.stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("Couldnt set read timeout");
            let mut hs_resp = [0; 128]; // needs to be more then 64
            let read_result = self.stream.read(&mut hs_resp);
            if let Ok(bytes_read) = read_result {
                let handshake_response = parse_handshake_response(&hs_resp.to_vec());

                // verify response is accurate
                if handshake_response.protocol != "BitTorrent protocol" {
                    return Err(err_msg(format!("Handshake protocol incorrect: {}", handshake_response.protocol)));
                }

                if handshake_response.info_hash != self.info_hash {
                    return Err(err_msg("Handshake Info hashes dont match!"));
                }

                // handshake is fine, start listening for have message
                println!("success!");
                Ok(())
            } else {
                Err(err_msg("handshake response failure."))
            }
        } else {
            Err(err_msg("handshake request failure."))
        }
    }
}

pub fn parse_peer_msg(buf: &[u8]) -> Option<PeerMessage> {
    // get size
    let msg_size = get_u32_at(buf, 0);

    // get id
    let id = buf[4];
    // keep-alive: <len=0000>
    // choke: <len=0001><id=0>
    // unchoke: <len=0001><id=1>
    // interested: <len=0001><id=2>
    // not interested: <len=0001><id=3>
    // have: <len=0005><id=4><piece index>
    // bitfield: <len=0001+X><id=5><bitfield>
    // request: <len=0013><id=6><index><begin><length>
    // piece: <len=0009+X><id=7><index><begin><block>
    // cancel: <len=0013><id=8><index><begin><length>
    // port: <len=0003><id=9><listen-port>
    match id {
        0 => {
            Some(PeerMessage::Choke(msg_size))
        }
        1 => {
            Some(PeerMessage::Unchoke(msg_size))
        }
        2 => {
            Some(PeerMessage::Interested(msg_size))
        }
        3 => {
            Some(PeerMessage::NotInterested(msg_size))
        }
        4 => {
            Some(PeerMessage::Have(msg_size, 0)) // TODO parse have piece
        }
        5 => {
            let (bitfield, new_i) = get_n_bytes_at(&buf.to_vec(), 5, (msg_size - 1) as usize);
            print_byte_array("bitfield", &bitfield);
            Some(PeerMessage::Bitfield(msg_size, bitfield))
        }
        6 => {
            Some(PeerMessage::Request(msg_size, 0, 0, 0)) // TODO parse requested piece
        }
        7 => {
            let piece_index = get_u32_at(&buf, 5);
            let piece_offset = get_u32_at(&buf, 9);

            // if buf is shorter then 13, doesnt have any data
            let data = if buf.len() > 13 {
                Some(get_bytes_til_end(&buf, 13))
            } else {
                None
            };

            Some(PeerMessage::Piece(msg_size, piece_index, piece_offset, data))
        }
        8 => {
            Some(PeerMessage::Cancel(msg_size))
        }
        9 => {
            Some(PeerMessage::Port(msg_size))
        }
        _ => {
            println!("unknown message id: {}", id);
            None
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_piece_data() {
        let buf =  [0x0, 0x0, 0x2, 0x9, 0x7, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x3C, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1F, 0x40, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x3, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0];
        let result = parse_peer_msg(&buf);
        assert!(result.is_some());
        let msg = result.unwrap();
        match msg {
            PeerMessage::Piece(id, piece_index, piece_offset, data) => {
                println!("piece index is: {}, offset is: {}", piece_index, piece_offset);
                assert_eq!(piece_index, 0);
                assert_eq!(piece_offset,15360); // validate this is correct

                assert_eq!(data.is_some(), true);
                let data = data.unwrap();

                print_byte_array("data", &data);
                println!("data is {} bytes", data.len());
                assert!(!data.is_empty());
                assert_eq!(data.len(), 512);
            },
            _ => {
                panic!("Should have been a piece message");
            }
        }
    }

    #[test]
    fn test_slice_data_buffer() {
        // buf has message in first 13 values, then 0's
        let bytes_read = 13;
        let buf =  [0x0, 0x0, 0x2, 0x9, 0x7, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x3C, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1F, 0x40, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x3, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0];

        let sliced_buf = &buf[..bytes_read];
        print_byte_array("sliced", sliced_buf);
    }
}