use rand::{AsByteSliceMut, Rng};
use std::io::{Cursor, Read, Write};
use std::thread;

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder};
use failure::err_msg;
use failure::Error;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::str::from_utf8;
use std::time::Duration;
use crossbeam::crossbeam_channel::{Receiver, Sender};

use crate::download::{Block, WorkChunk};
use crate::*;
use std::collections::VecDeque;
use crate::peer::PeerMessage::KeepAlive;

pub fn attempt_peer_connect(
    ip: IpAddr,
    port: u16,
    info_hash_array: &[u8; 20],
    peer_id: &[u8; 20],
) -> Result<Peer, Error> {
    let mut peer = Peer::new(
        ip.clone(),
        port.clone(),
        info_hash_array.clone(),
        peer_id.clone()
    );

    // connect
    if let Err(e) = peer.connect() {
        println!("Error connecting {:?} = {:?}", peer.ip, e);
        return Err(err_msg(format!("error connecting: {:?}", e)))
    }
    println!("connected to {:?}", peer.ip);

    // handshake
    if let Err(e) = peer.perform_handshake() {
        println!("Error handshake for ip: {:?}", peer.ip);
        return Result::Err(err_msg(e));
    }
    println!("Handshaked with {:?}", peer.ip);

    // rcv bitfield
    peer.recv_garbage();

    // send interested
    peer.send_interested();
    println!("send interested to peer: {:?}", peer.ip);

    if let Err(e) = peer.recv_unchoke() {
        println!("Error unchoking:{:?} = {:?}", peer.ip, e)
        // TODO should return error
    }

    Ok(peer)
}

// higher level function, tries connecting to peer, handshake, and start downloading data
pub fn attempt_peer_download(
    mut peer: Peer,
    work_sender: &Sender<WorkChunk>,
    work_recv: &Receiver<WorkChunk>,
    processing_chan: &Sender<Block>,
) -> Result<(), Error> {

    // start pulling work off work queue
    loop {
        match work_recv.recv() {
            Ok(next_chunk) => {
                println!(
                    "> DL {}:{}...",
                    next_chunk.piece_index, next_chunk.block_id
                );
                match peer.fetch_block_data(&next_chunk) {
                    Ok(data) => {
                        // print_byte_array("piece data", &piece_data);
                        let block = Block {
                            data,
                            piece_index: next_chunk.piece_index,
                            offset: next_chunk.begin_index,
                            block_id: next_chunk.block_id,
                        };

                        if let Err(e) = processing_chan.send(block) {
                            println!("Error fetching block: {}-{} {:?} adding back to work channel",next_chunk.piece_index, next_chunk.block_id, e);
                            work_sender.send(next_chunk)?;
                            break;
                        }
                    }
                    Err(e) => {
                        println!("Error fetching block: {}-{} {:?} adding back to work channel",next_chunk.piece_index, next_chunk.block_id, e);
                        work_sender.send(next_chunk)?;
                        break;
                    }
                }
            }
            Err(e) => {
                println!("Error recv on work queue: {:?}", e);
                break;
            }
        }
    }
    // at this point, the connection ended successfully
    return Result::Ok(());
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
    Choke(u32),                            // id = 0
    Unchoke(u32),                          // id = 1
    Interested(u32),                       // id = 2
    NotInterested(u32),                    // id = 3
    Have(u32, u32),                        // id = 4, piece_index=4bytes
    Bitfield(u32, Vec<u8>),                // id = 5, bitfield
    Request(u32, u32, u32, u32),           // id = 6, index=4, begin=4, length=4
    Piece(u32, u32, u32, Option<Vec<u8>>), // id = 6, index=4, offset/begin=4, vector of piece data
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

#[derive(Debug)]
pub struct Peer {
    choked: bool,
    stream: Option<TcpStream>,
    ip: IpAddr,
    port: u16,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    bitfield: Option<Vec<u8>>,
}

impl Peer {
    // Makes new peer and connects
    pub fn new(
        ip: IpAddr,
        port: u16,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
    ) -> Self {
        Peer {
            choked: false,
            stream: None,
            ip,
            port,
            info_hash,
            peer_id,
            bitfield: None,
        }
    }

    pub fn connect(&mut self) -> Result<(), Error>{
        let ip = self.ip;
        let port = self.port;

        let sock_addr = SocketAddr::new(ip, port);
        let stream = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(2))?;
        // set stream to blocking
        stream
            .set_nonblocking(false)
            .expect("cant set stream to blocking");
        // set stream on peer
        self.stream = Some(stream);
        Ok(())
   }

    // for some reason after handshake theres a bunch of nonesense sent, read it out of queue
    pub fn recv_garbage(&mut self) {
        let mut buf = [0; 512];
        let read_result = self.stream.as_ref().unwrap().read(&mut buf);
        match read_result {
            Ok(bytes_read) => {
                let header = format!("peer {:?} bitfield", self.ip);
                println!("got {}", header);
                // print_byte_array_len(header.as_str(), &buf, bytes_read);
            }
            Err(e) => {}
        }
    }

    pub fn maybe_revc_bitfield(&mut self) {
        // read 4 byte size
        let mut buf = [0; 4];
        let read_result = self.stream.as_ref().unwrap().read(&mut buf);
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
                let read_result = self.stream.as_ref().unwrap().read(&mut buf);

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
        let msg = make_interested_msg();
        self.stream.as_ref().unwrap().write_all(&msg).unwrap();
    }

    pub fn recv_unchoke(&mut self) -> Result<(), Error> {
        let mut buf = [0; 512];
        let bytes_read = self.stream.as_ref().unwrap().read(&mut buf)?;
        match parse_peer_msg(&buf)? {
            PeerMessage::Unchoke(_) => {
                self.choked = false;
                Ok(())
            }
            _ => {
                // println!("received msg other then choke! No good");
                print_byte_array("unchoke msg", &buf);
                Err(err_msg("received msg other then choke"))
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

        // write request to stream
        let req = make_request_msg(piece_id, offset, length);
        self.stream.as_ref().unwrap().write_all(&req).expect("write all failed"); //TODO error handle correctly

        // set longer read timeout in case of slow connection
        self.stream
            .as_ref()
            .unwrap()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        // since it may take multiple messages to send all data, need to concat to this buffer
        let mut all_data = Vec::new();

        // keep reading stream until all data for block has been received
        let mut i = 0;
        let mut total_msg_size: isize = 0; // is set on first read
        let mut total_bytes_read: isize = 0;
        loop {
            let mut read_buf = [0; 5000];
            let bytes_read = self.stream.as_ref().unwrap().read(&mut read_buf).expect("read filed"); // TODO error handle correctly

            if i == 0 {
                // slice only section of buf written to
                let sliced_buf = &read_buf[..bytes_read];
                let msg = parse_peer_msg(sliced_buf)?;
                if let PeerMessage::Piece(msg_size, piece_id, _offset, data) = msg {
                    total_msg_size = msg_size as isize;
                    total_bytes_read += bytes_read as isize;

                    if let Some(d) = data {
                        all_data.extend(d);
                    }
                } else {
                    return Err(err_msg(
                        "Peer message was not a piece msg as expected! returning error",
                    ));
                }
            } else {
                // second or more reads, append data to buffer
                total_bytes_read += bytes_read as isize;
                let remaining_to_read = total_msg_size - total_bytes_read;

                // append data
                let sliced_buf = &read_buf[..bytes_read];
                all_data.extend_from_slice(sliced_buf);
                if remaining_to_read <= 0 {
                    return Ok(all_data);
                }
            }
            // // wait a sec then read again
            // thread::sleep(Duration::from_secs(1));
            i += 1;
        }
    }

    pub fn perform_handshake(&mut self) -> Result<(), Error> {
        debug(format!("performing handshake {:?}", self.ip));
        let handshake = make_handshake(&self.peer_id, &self.info_hash);

        // write handshake to stream
        let write_result = self.stream.as_ref().unwrap().write_all(&handshake);
        println!("wrote handshake to  {:?}", self.ip);


        if let Ok(bytes_wrote) = write_result {
            // set read timeout
            self.stream
                .as_ref()
                .unwrap()
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("Couldnt set read timeout");
            let mut hs_resp = [0; 128]; // needs to be more then 64
            let read_result = self.stream.as_ref().unwrap().read(&mut hs_resp);
            if let Ok(bytes_read) = read_result {
                println!("read {:?} bytes from handshake {:?}", bytes_read, self.ip);
                let handshake_response = parse_handshake_response(&hs_resp.to_vec());
                println!("handshake response: {:?}", handshake_response);

                // verify response is accurate
                if handshake_response.protocol != "BitTorrent protocol" {
                    println!("Handshake protocol isnt bittorrent protocol");
                    return Err(err_msg(format!(
                        "Handshake protocol incorrect: {}",
                        handshake_response.protocol
                    )));
                }

                if handshake_response.info_hash != self.info_hash {
                    println!("handshake info hash doesnt match our info hash");
                    return Err(err_msg("Handshake Info hashes dont match!"));
                }

                // handshake is fine, start listening for have message
                Ok(())
            } else {
                Err(err_msg("handshake response failure."))
            }
        } else {
            println!("handshake write failed");
            Err(err_msg("handshake request failure."))
        }
    }
}

pub fn parse_peer_msg(buf: &[u8]) -> Result<PeerMessage, Error> {
    // get size
    let msg_size = get_u32_at(buf, 0);
    if msg_size <= 4 {
        // keep alive
        return Ok(PeerMessage::KeepAlive)
    }

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
        0 => Ok(PeerMessage::Choke(msg_size)),
        1 => Ok(PeerMessage::Unchoke(msg_size)),
        2 => Ok(PeerMessage::Interested(msg_size)),
        3 => Ok(PeerMessage::NotInterested(msg_size)),
        4 => {
            Ok(PeerMessage::Have(msg_size, 0)) // TODO parse have piece
        }
        5 => {
            let (bitfield, new_i) = get_n_bytes_at(&buf.to_vec(), 5, (msg_size - 1) as usize);
            Ok(PeerMessage::Bitfield(msg_size, bitfield))
        }
        6 => {
            Ok(PeerMessage::Request(msg_size, 0, 0, 0)) // TODO parse requested piece
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

            Ok(PeerMessage::Piece(
                msg_size,
                piece_index,
                piece_offset,
                data,
            ))
        }
        8 => Ok(PeerMessage::Cancel(msg_size)),
        9 => Ok(PeerMessage::Port(msg_size)),
        _ => Err(err_msg(format!("unknown msg id: {}", id))),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_piece_data() {
        let buf = [
            0x0, 0x0, 0x2, 0x9, 0x7, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x3C, 0x0, 0x1, 0x0, 0x0, 0x3,
            0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7,
            0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8,
            0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1F, 0x40, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0,
            0x0, 0x0, 0x3, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0,
            0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0,
            0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1,
            0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0,
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0,
            0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58,
            0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0,
            0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1,
            0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0,
            0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0,
            0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13,
            0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0,
            0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0,
            0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1,
            0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0,
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0,
            0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58,
            0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0,
            0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0,
        ];
        let result = parse_peer_msg(&buf);
        assert!(result.is_ok());
        let msg = result.unwrap();
        match msg {
            PeerMessage::Piece(id, piece_index, piece_offset, data) => {
                println!(
                    "piece index is: {}, offset is: {}",
                    piece_index, piece_offset
                );
                assert_eq!(piece_index, 0);
                assert_eq!(piece_offset, 15360); // validate this is correct

                assert_eq!(data.is_some(), true);
                let data = data.unwrap();

                print_byte_array("data", &data);
                println!("data is {} bytes", data.len());
                assert!(!data.is_empty());
                assert_eq!(data.len(), 512);
            }
            _ => {
                panic!("Should have been a piece message");
            }
        }
    }

    #[test]
    fn test_slice_data_buffer() {
        // buf has message in first 13 values, then 0's
        let bytes_read = 13;
        let buf = [
            0x0, 0x0, 0x2, 0x9, 0x7, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x3C, 0x0, 0x1, 0x0, 0x0, 0x3,
            0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7,
            0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8,
            0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1F, 0x40, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0,
            0x0, 0x0, 0x3, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0,
            0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0,
            0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1,
            0x0, 0x0, 0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0,
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0,
            0x1B, 0x58, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58,
            0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58, 0x0, 0x0,
            0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1,
            0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0,
            0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0,
            0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13,
            0x88, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x13, 0x88, 0x0,
            0x0, 0x0, 0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xF, 0xA0, 0x0, 0x0, 0x0,
            0x1, 0x0, 0x0, 0x7, 0xD0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1,
            0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0,
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0,
            0x17, 0x70, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1B, 0x58,
            0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x2, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0,
            0x0, 0x1, 0x0, 0x0, 0xB, 0xB8, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
            0x1, 0x0, 0x0, 0x3, 0xE8, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x17, 0x70, 0x0, 0x0, 0x0,
        ];

        let sliced_buf = &buf[..bytes_read];
        print_byte_array("sliced", sliced_buf);
    }
}
