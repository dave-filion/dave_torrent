use rand::{AsByteSliceMut, Rng};
use std::io::{Cursor, Read, Write};

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use std::fmt::Error;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::str::from_utf8;
use std::time::Duration;

use crate::download::WorkChunk;
use crate::*;
use std::collections::VecDeque;

// higher level function, tries connecting to peer, handshake, and start downloading data
pub fn attempt_peer_download(
    ip: IpAddr,
    port: u16,
    info_hash_array: &[u8; 20],
    peer_id: &[u8; 20],
    work_queue: &mut VecDeque<WorkChunk>,
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
        println!("Error connecting to peer: {:?}", e);
        return Result::Err(Error);
    }

    let mut peer = peer_result.unwrap();
    let handshake_result = peer.perform_handshake();
    if let Err(e) = handshake_result {
        println!("Error performing handshake: {:?}", e);
        return Result::Err(Error);
    }

    // actually not garbage, is bitfield
    // TODO recv bitfield
    peer.recv_garbage();
    peer.send_interested();

    if peer.recv_choke() {
        // start pulling work off work queue
        println!("Peer unchoked...Starting to pull work off work queue");
        loop {
            match work_queue.pop_front() {
                Some(next_chunk) => {
                    println!("Next chunk to download: {:?}", next_chunk);
                    let piece_result = peer.request_piece(
                        next_chunk.piece_index,
                        next_chunk.begin_index,
                        next_chunk.length,
                    );

                    match piece_result {
                        Ok(piece_data) => {
                            println!("Got piece data!");
                            print_byte_array("piece data", &piece_data);
                            // put on channel to be processed and go to next one
                        },
                        Err(e) => {
                            println!("Error getting piece data, putting chunk back on queue");
                            work_queue.push_back(next_chunk);
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
        println!("Couldnt unchoke peer, trying next");
        return Result::Err(Error);
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
        Result::Err(Error)
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
    Choke(usize),                  // id = 0
    Unchoke(usize),                // id = 1
    Interested(usize),             // id = 2
    NotInterested(usize),          // id = 3
    Have(usize, u32),              // id = 4, piece_index=4bytes
    Bitfield(usize, Vec<u8>),      // id = 5, bitfield
    Request(usize, u32, u32, u32), // id = 6, index=4, begin=4, length=4
    Piece(usize, Vec<u8>),         // id = 6, vector of piece data
    Cancel(usize),
    Port(usize),
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
            println!("Couldnt connect to peer: {:?}:{}", ip, port);
            Result::Err(Error)
        }
    }

    // for some reason after handshake theres a bunch of nonesense sent, read it out of queue
    pub fn recv_garbage(&mut self) {
        println!("reading garbage from peer");
        let mut buf = [0; 512];
        let read_result = self.stream.read(&mut buf);
        match read_result {
            Ok(bytes_read) => {
                println!("Read {} bytes of crap from peer. looks like...", bytes_read);
                print_byte_array_len("crap", &buf, bytes_read);
            }
            Err(e) => {
                println!("error reading crap from peer: {:?}", e);
            }
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
        println!("Sending interested message to peer");
        let msg = make_interested_msg();
        self.stream.write_all(&msg).unwrap();
    }

    pub fn recv_choke(&mut self) -> bool {
        // wait 5 secs for choke msg
        self.stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("Cant set read timeout");
        println!("Waiting for choke message");
        let mut buf = [0; 512];
        let read_result = self.stream.read(&mut buf);
        match read_result {
            Ok(bytes) => {
                println!("read {} bytes", bytes);
                print_byte_array_len("choke msg", &buf, bytes); // only print til bytes read
                println!("parsing peer msg");
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
    pub fn request_piece(&mut self, piece: u32, begin: u32, len: u32) -> Result<Vec<u8>, Error> {
        println!(
            "Sending request message for piece: {}, begin: {}, len: {}",
            piece, begin, len
        );
        let req = make_request_msg(piece, begin, len);
        self.stream.write_all(&req).expect("Write request failed");

        // listen for response
        println!("Waiting for piece response");
        self.stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let mut buf = [0; 1028]; // 1028 chunks
        match self.stream.read(&mut buf) {
            Ok(bytes_read) => {
                println!("Read {} bytes for piece", bytes_read);
                print_byte_array_len("piece result", &buf, bytes_read);
                if let Some(piece_msg) = parse_peer_msg(&buf) {
                    if let PeerMessage::Piece(_id, piece_data) = piece_msg {
                        Result::Ok(piece_data)
                    } else {
                        println!("Msg recv was NOT piece, instead was: {:?}", piece_msg);
                        Result::Err(Error)
                    }
                } else {
                    println!("no peerMsg could be parsed");
                    Result::Err(Error)
                }
            }
            Err(e) => {
                println!("error reading piece after request {:?}", e);
                Result::Err(Error)
            }
        }
    }

    pub fn perform_handshake(&mut self) -> Result<(), Error> {
        println!("Peer at {:?} performing handshake...", self.ip);
        let handshake = make_handshake(&self.peer_id, &self.info_hash);

        // write handshake to stream
        let write_result = self.stream.write_all(&handshake);
        if let Ok(bytes_wrote) = write_result {
            println!("Waiting for response from {:?}:{}", self.ip, self.port);
            // set read timeout
            self.stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("Couldnt set read timeout");
            let mut hs_resp = [0; 128]; // needs to be more then 64
            let read_result = self.stream.read(&mut hs_resp);
            if let Ok(bytes_read) = read_result {
                println!(
                    "Recieved {} byte handshake response from {:?}:{}",
                    bytes_read, self.ip, self.port
                );
                // parse response
                // print_byte_array("handshake response", &resp_buf);
                let handshake_response = parse_handshake_response(&hs_resp.to_vec());
                // verify response is accurate
                println!("verifying handshake from {:?}:{}", self.ip, self.port);
                if handshake_response.protocol != "BitTorrent protocol" {
                    println!("Protocol incorrect: {}", handshake_response.protocol);
                    return Result::Err(Error);
                } else {
                    println!("...protocol OK")
                }

                if handshake_response.info_hash != self.info_hash {
                    println!("Info hashes dont match... going to next peer");
                    return Result::Err(Error);
                } else {
                    println!("...info hash OK");
                }

                // handshake is fine, start listening for have message
                println!(
                    "Handshake to {:?}:{} SUCCESS, listening for messages",
                    self.ip, self.port
                );

                Result::Ok(())
            } else {
                println!(
                    "Reading handshake response from stream failed for {:?}:{}",
                    self.ip, self.port
                );
                Result::Err(Error)
            }
        } else {
            println!(
                "Writing handshake to stream failed for {:?}:{}",
                self.ip, self.port
            );
            Result::Err(Error)
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
            println!("choke message recv");
            Some(PeerMessage::Choke(0))
        }
        1 => {
            println!("unchoke message rcv");
            Some(PeerMessage::Unchoke(1))
        }
        2 => {
            println!("interested message rcv");
            Some(PeerMessage::Interested(2))
        }
        3 => {
            println!("not interested msg recv");
            Some(PeerMessage::NotInterested(3))
        }
        4 => {
            println!("have msg recv");
            Some(PeerMessage::Have(4, 0)) // TODO parse have piece
        }
        5 => {
            println!("bitfield msg recv");
            println!("contains {} bits of data", (msg_size - 1) * 8);
            let (bitfield, new_i) = get_n_bytes_at(&buf.to_vec(), 5, (msg_size - 1) as usize);
            print_byte_array("bitfield", &bitfield);
            Some(PeerMessage::Bitfield(5, bitfield))
        }
        6 => {
            println!("request msg recv");
            Some(PeerMessage::Request(6, 0, 0, 0)) // TODO parse requested piece
        }
        7 => {
            println!("piece msg recv");
            Some(PeerMessage::Piece(7, Vec::new())) // TODO parse piece data
        }
        8 => {
            println!("cancelled msg recv");
            Some(PeerMessage::Cancel(8))
        }
        9 => {
            println!("port msg recv");
            Some(PeerMessage::Port(9))
        }
        _ => {
            println!("unknown message id: {}", id);
            None
        }
    }
}
