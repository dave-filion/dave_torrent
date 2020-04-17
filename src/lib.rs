use rand::{AsByteSliceMut, Rng};
use std::io::{Cursor, Read, Write};

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use std::fmt::Error;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::str::from_utf8;
use std::time::Duration;

#[derive(Debug)]
pub struct AnnounceResponse {
    pub action: u32, // usually 1
    pub transaction_id: u32,
    pub interval: u32,
    pub leechers: u32,
    pub seeders: u32,
    pub addresses: Vec<(IpAddr, u16)>, // vector of tuples (addr, port) of peers
}

impl From<Vec<u8>> for AnnounceResponse {
    fn from(buf: Vec<u8>) -> Self {
        parse_announce_response(&buf)
    }
}

pub fn print_byte_array(header: &str, bytes: &[u8]) {
    print!("{} => [", header);
    for (i, b) in bytes.iter().enumerate() {
        if i == 0 {
            print!("0x{:X?}", b);
        } else {
            print!(", 0x{:X?}", b);
        }
    }
    println!("]");
}

pub fn make_connect_packet() -> Vec<u8> {
    let mut bytebuf = ByteBuffer::new();

    // write connection id
    let conn_id: u64 = 0x41727101980; // constant for connecting
    bytebuf.write_u64(conn_id);

    // write action = 0
    let action: u32 = 0;
    bytebuf.write_u32(action);

    // write random tx id (4 bytes)
    let random_tx_id = rand::thread_rng().gen::<[u8; 4]>();
    bytebuf.write_bytes(&random_tx_id);

    bytebuf.to_bytes()
}

pub fn get_conn_id_from_connect_response(response_buf: &[u8]) -> (u64, Vec<u8>) {
    // get connection id from response (at index 7, 8 bytes)
    let mut conn_id_buff = [0; 8];
    let mut j = 0;
    for i in 8..16 {
        let byte = response_buf[i];
        conn_id_buff[j] = byte;
        j += 1;
    }
    // convert conn id to u64
    let mut byte_rdr = Cursor::new(conn_id_buff);
    (
        byte_rdr.read_u64::<BigEndian>().unwrap(),
        conn_id_buff.to_vec(),
    )
}

pub fn make_announce_packet(
    conn_id_bytes: &Vec<u8>,
    info_hash_bytes: &Vec<u8>,
    torrent_size: u64,
    port: u16,
    peer_id: &[u8; 20],
    tx_id: &[u8; 4],
) -> Vec<u8> {
    let mut buf = ByteBuffer::new();

    // conn id (8 bytes)
    buf.write_bytes(&conn_id_bytes);

    // action (1 for announce, 4 bytes)
    buf.write_u32(1);

    // transaction_id (random 4 bytes)
    // let tx_id = rand::thread_rng().gen::<[u8; 4]>();
    // print_byte_array("random tx id", &tx_id);
    buf.write_bytes(tx_id);

    // info hash (20 bytes)
    buf.write_bytes(&info_hash_bytes);

    // peer id (random 20 bytes)
    // let peer_id = rand::thread_rng().gen::<[u8; 20]>();
    // print_byte_array("random peer id", &peer_id);
    buf.write_bytes(peer_id);

    // downloaded (8 bytes, just zeros here)
    buf.write_bytes(&[0; 8]);

    // left (8 bytes) size of torrent
    buf.write_u64(torrent_size);

    // uploaded (8 bytes, just zeros again)
    buf.write_bytes(&[0; 8]);

    // event (4 byte)
    buf.write_u32(0);

    // ip address (4 byte, also 0)
    buf.write_u32(0);

    // key (4 bytes, random)
    let key = rand::thread_rng().gen::<[u8; 4]>();
    print_byte_array("key", &key);
    buf.write_bytes(&key);

    // num_want (4 bytes, -1 is default)
    buf.write_i32(-1);

    // port (2 bytes)
    buf.write_u16(port);

    buf.to_bytes()
}

pub fn get_socket_addr(announce_url: &str) -> SocketAddr {
    let pieces: Vec<String> = announce_url.split("://").map(|s| s.to_string()).collect(); // seperates protocol from url
    let protocol = pieces[0].as_str();
    if protocol != "udp" {
        panic!("cant handle non udp protocol: {}", protocol);
    }

    let url = pieces[1].as_str();
    url.to_socket_addrs().unwrap().next().unwrap()
}

pub fn send_connect_req(sock: &UdpSocket) -> Vec<u8> {
    let connect_packet = make_connect_packet();
    //print_byte_array("Connect request", &connect_packet);

    // send message to remote udp port
    let _result = sock.send(&connect_packet);
    println!("Send conn req... waiting for response...");

    // TODO: retry logic
    // listen for response
    let mut response_buf = [0; 16];
    let _num_bytes = sock.recv(&mut response_buf).expect("Didnt recieve data");
    //print_byte_array("Response", &response_buf);

    // extract conn id
    let (_conn_id_int, conn_id_bytes) = get_conn_id_from_connect_response(&response_buf);
    conn_id_bytes
}

pub fn send_announce_req(
    sock: &UdpSocket,
    conn_id_bytes: &Vec<u8>,
    info_hash_bytes: &Vec<u8>,
    torrent_size: u64,
    port: u16,
    peer_id: &[u8; 20],
    tx_id: &[u8; 4],
) -> AnnounceResponse {
    // now send announce message and return response
    let announce_packet = make_announce_packet(
        &conn_id_bytes,
        &info_hash_bytes,
        torrent_size,
        port,
        peer_id,
        tx_id,
    );
    //print_byte_array("Announce", &announce_packet);

    let _result = sock.send(&announce_packet);
    println!("Sent announce packet... waiting for response...");

    // listen for response TODO: need to implement timeout and retry
    let mut response_buf = [0; 128];
    let _num_bytes = sock.recv(&mut response_buf).unwrap();
    //print_byte_array("announce resp", &response_buf);

    // transform into announce response
    response_buf.to_vec().into()
}

pub fn get_u32_at(from: &[u8], index: usize) -> u32 {
    let mut buf = [0; 4];
    let mut j = 0;
    for i in index..index + 4 {
        let b = from[i];
        buf[j] = b;
        j += 1;
    }
    // conert from bytes to u32
    BigEndian::read_u32(&buf)
}

pub fn parse_announce_response(resp: &Vec<u8>) -> AnnounceResponse {
    // 0  - 4b - action (1)
    // 4  - 4b - transaction_id
    // 8  - 4b - interval
    // 12 - 4b - leechers
    // 16 - 4b - seeders
    // 20 + 6 * n - 4b - ip address
    // 24 + 6 * n - 2b - TCP port

    // go to byte 4 to get transaction id
    let transaction_id = get_u32_at(resp, 4);

    let interval = get_u32_at(resp, 8);

    let leechers = get_u32_at(resp, 12);

    let seeders = get_u32_at(resp, 16);

    // at offset 20, there are 6 byte units, first 4 are ip addr, 2 are port
    let mut i = 20;
    let mut addresses = Vec::new();
    loop {
        let maybe_result = resp.get(i);
        if maybe_result.is_none() {
            // end of list, break
            break;
        } else {
            // get next 4 bytes, for addr
            let mut ip_addr = [0; 4];
            let mut j = 0;
            for k in i..i + 4 {
                let b = resp.get(k).unwrap().clone();
                ip_addr[j] = b;
                j += 1;
            }
            // print_byte_array("ipaddr", &ip_addr);

            //get next 2 bytes for port
            let mut port = [0; 2];
            let mut j = 0;
            for k in i + 4..i + 6 {
                let b = resp.get(k).unwrap().clone();
                port[j] = b;
                j += 1;
            }
            // print_byte_array("port", &port);

            // transform from bytes to ints
            let port = BigEndian::read_u16(&port);

            // if ip_addr is 0, we reached the end, break\
            if ip_addr[0] == 0 {
                break;
            }

            // translate to object
            let ip_addr: IpAddr = ip_addr.into();

            addresses.push((ip_addr, port));

            // advance i
            i += 6;
        }
    }

    AnnounceResponse {
        action: 1, // always 1
        transaction_id,
        interval,
        leechers,
        seeders,
        addresses,
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

// reads n bytes from buf at index and returns vec, returns new index also
pub fn get_n_bytes_at(buf: &Vec<u8>, start_i: usize, n_bytes: usize) -> (Vec<u8>, usize) {
    let mut v = Vec::new();
    let mut j = start_i;
    for i in start_i..start_i + n_bytes {
        let byte = buf.get(i).unwrap();
        v.push(byte.clone());
        j = i;
    }
    (v, j)
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
pub enum PeerMessage {
    KeepAlive,
    Choke(usize),             // id = 0
    Unchoke(usize),           // id = 1
    Interested(usize),        // id = 2
    NotInterested(usize),     // id = 3
    Have(usize, u32),         // id = 4, piece_index=4bytes
    Bitfield(usize, [u8; 4]), // id = 5, bitfield=4bytes
    Request(usize, u32, u32), // id = 6, index=4, begin=4, length=4
    Piece(usize),             // TODO should have more
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

fn make_uninterested_msg() -> Vec<u8> {
    let mut buf = ByteBuffer::new();
    // length
    buf.write_u32(1);
    // id
    buf.write_u8(3);

    buf.to_bytes()
}

fn make_have_msg(piece_index: u32) -> Vec<u8> {
    let mut buf = ByteBuffer::new();
    // length
    buf.write_u32(5);
    // id
    buf.write_u8(4);
    // piece index
    buf.write_u32(piece_index);

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
        if let Ok(stream) = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5)) {
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

    pub fn perform_handshake(&mut self) -> Result<(), Error> {
        println!("Peer at {:?} performing handshake...", self.ip);
        let handshake = make_handshake(&self.peer_id, &self.info_hash);

        // write handshake to stream
        let write_result = self.stream.write_all(&handshake);
        if let Ok(bytes_wrote) = write_result {
            println!("Waiting for response from {:?}:{}", self.ip, self.port);
            // set read timeout
            self.stream.set_read_timeout(Some(Duration::from_secs(5))).expect("Couldnt set read timeout");
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

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::from_utf8;

    #[test]
    fn test_make_announce_packet() {
        let port = 6969;
        let torrent_size = 1; // 1 so its easy to see in byte form
        let peer_id = rand::thread_rng().gen::<[u8; 20]>();
        let tx_id = [0x10, 0x20, 0x30, 0x40];
        let conn_id_bytes = [0x10, 0x38, 0x94, 0xC3, 0x73, 0x6B, 0x76, 0xB2].to_vec();
        let info_hash_bytes = [
            0xAA, 0x16, 0x30, 0x38, 0x78, 0x53, 0x79, 0x81, 0x90, 0x75, 0x43, 0x56, 0x11, 0x51,
            0x73, 0x33, 0x45, 0x89, 0x19, 0xCC,
        ]
        .to_vec();
        let result = make_announce_packet(
            &conn_id_bytes,
            &info_hash_bytes,
            torrent_size,
            port,
            &peer_id,
            &tx_id,
        );

        print_byte_array("result", &result);
        assert_eq!(result.len(), 98);
    }

    #[test]
    fn test_parse_announce_response() {
        let sample_announce_resp = [
            0x0, 0x0, 0x0, 0x1, 0x65, 0x48, 0x92, 0x2D, 0x0, 0x0, 0x6, 0xB8, 0x0, 0x0, 0x0, 0x1,
            0x0, 0x0, 0x0, 0x9, 0x42, 0x6C, 0x62, 0x33, 0x85, 0xD8, 0xDB, 0x5B, 0x8B, 0xEB, 0xE8,
            0x74, 0xDB, 0x5B, 0x8B, 0xEB, 0xC8, 0x54, 0xD8, 0x24, 0xF, 0x65, 0xC8, 0xD5, 0xD0,
            0x48, 0xC0, 0xE5, 0x83, 0x1E, 0x54, 0x11, 0x35, 0xA9, 0xC8, 0xD5, 0x4F, 0x58, 0xB2,
            0x94, 0x60, 0x87, 0x4A, 0x3A, 0x73, 0x24, 0xBF, 0x7F, 0x47, 0x3A, 0xFC, 0x7A, 0x1E,
            0xC9, 0x44, 0xA8, 0xB2, 0x15, 0xC8, 0xD5,
        ]
        .to_vec();

        let result = parse_announce_response(&sample_announce_resp);
        println!("result => {:?}", result);
        assert_eq!(result.action, 1);
        assert_eq!(result.leechers, 1);
        assert_eq!(result.seeders, 9);

        // try using From
        let result: AnnounceResponse = sample_announce_resp.into();
        println!("result => {:?}", result);
        assert_eq!(result.action, 1);
        assert_eq!(result.leechers, 1);
        assert_eq!(result.seeders, 9);

        // check addresses
        assert_eq!(result.addresses.len(), 10);
    }

    #[test]
    fn test_get_u32_at_position() {
        let buffer = [0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0];
        let result = get_u32_at(&buffer, 4);
        assert_eq!(result, 2);

        let result = get_u32_at(&buffer, 0);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_make_handshake() {
        // make random 20 byte peer id and info hash
        let peer_id = rand::thread_rng().gen::<[u8; 20]>();
        let info_hash = rand::thread_rng().gen::<[u8; 20]>();

        let packet = make_handshake(&peer_id, &info_hash);
        print_byte_array("handshake", &packet);
        print_byte_array("peer_id", &peer_id);
        let pstr = [
            0x42, 0x69, 0x74, 0x54, 0x6F, 0x72, 0x72, 0x65, 0x6E, 0x74, 0x20, 0x70, 0x72, 0x6F,
            0x74, 0x6F, 0x63, 0x6F, 0x6C,
        ];
        let s = from_utf8(&pstr).unwrap();
        println!("s => {}", s);
    }

    #[test]
    fn test_get_n_bytes_at() {
        let buf = [
            0x13, 0x42, 0x69, 0x74, 0x54, 0x6F, 0x72, 0x72, 0x65, 0x6E, 0x74, 0x20, 0x70, 0x72,
            0x6F, 0x74, 0x6F, 0x63, 0x6F, 0x6C, 0x0, 0x0, 0x0, 0x0, 0x0, 0x10, 0x0, 0x5, 0x20,
            0x9C, 0x82, 0x26, 0xB2, 0x99, 0xB3, 0x8, 0xBE, 0xAF, 0x2B, 0x9C, 0xD3, 0xFB, 0x49,
            0x21, 0x2D, 0xBD, 0x13, 0xEC, 0x2D, 0x54, 0x52, 0x32, 0x39, 0x34, 0x30, 0x2D, 0x74,
            0x65, 0x6A, 0x63, 0x67, 0x6D, 0x32, 0x6E, 0x74, 0x78, 0x74, 0x6F, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0,
        ]
        .to_vec();

        let (result, new_i) = get_n_bytes_at(&buf, 2, 7);
        print_byte_array("result", &result);
        assert_eq!(result.len(), 7);
        assert_eq!(result.get(0).unwrap(), &0x69);
        assert_eq!(result.get(6).unwrap(), &0x65);
        println!("new i = {}", new_i);
        assert_eq!(new_i, 8);
    }

    #[test]
    fn test_tcp_handshake_response() {
        let response = [
            0x13, 0x42, 0x69, 0x74, 0x54, 0x6F, 0x72, 0x72, 0x65, 0x6E, 0x74, 0x20, 0x70, 0x72,
            0x6F, 0x74, 0x6F, 0x63, 0x6F, 0x6C, 0x0, 0x0, 0x0, 0x0, 0x0, 0x10, 0x0, 0x5, 0x20,
            0x9C, 0x82, 0x26, 0xB2, 0x99, 0xB3, 0x8, 0xBE, 0xAF, 0x2B, 0x9C, 0xD3, 0xFB, 0x49,
            0x21, 0x2D, 0xBD, 0x13, 0xEC, 0x2D, 0x54, 0x52, 0x32, 0x39, 0x34, 0x30, 0x2D, 0x74,
            0x65, 0x6A, 0x63, 0x67, 0x6D, 0x32, 0x6E, 0x74, 0x78, 0x74, 0x6F, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0,
        ]
        .to_vec();

        let response = parse_handshake_response(&response);
        println!("response : {:?}", response);
        assert_eq!(response.protocol, "BitTorrent protocol".to_string());
        assert_eq!(
            response.info_hash,
            [
                0x20, 0x9C, 0x82, 0x26, 0xB2, 0x99, 0xB3, 0x8, 0xBE, 0xAF, 0x2B, 0x9C, 0xD3, 0xFB,
                0x49, 0x21, 0x2D, 0xBD, 0x13, 0xEC
            ]
            .to_vec()
        );
        assert_eq!(
            response.peer_id,
            [
                0x2D, 0x54, 0x52, 0x32, 0x39, 0x34, 0x30, 0x2D, 0x74, 0x65, 0x6A, 0x63, 0x67, 0x6D,
                0x32, 0x6E, 0x74, 0x78, 0x74, 0x6F
            ]
            .to_vec()
        );
    }

    #[test]
    fn connect_to_peer_live_test() {
        // List of 9 peers:
        // -> V4(66.108.98.51):34264
        // -> V4(219.91.135.231):60709
        // -> V4(216.36.15.101):51413
        // -> V4(208.72.192.229):33566
        // -> V4(84.17.53.169):51413
        // -> V4(79.88.178.148):24711
        // -> V4(74.58.115.36):49023
        // -> V4(71.58.252.122):7881
        // -> V4(68.168.178.21):51413
        let ip = Ipv4Addr::new(66, 108, 98, 51);
        let ip = IpAddr::V4(ip);
        let port = 34264;
        // connect_to_peer(ip, port);
    }

    #[test]
    fn test_make_choke_msg() {
        let msg = make_choke_msg();
        print_byte_array("choke", &msg);
        assert_eq!(msg.len(), 5); // should be 5 bytes long
        assert_eq!(msg.get(3).unwrap(), &1u8);
        assert_eq!(msg.get(4).unwrap(), &0u8);
    }

    #[test]
    fn test_make_unchoke_msg() {
        let msg = make_unchoke_msg();
        print_byte_array("unchoke", &msg);
        assert_eq!(msg.len(), 5); // should be 5 bytes long
        assert_eq!(msg.get(3).unwrap(), &1u8);
        assert_eq!(msg.get(4).unwrap(), &1u8);
    }

    #[test]
    fn test_make_have_msg() {
        let pi = 4;
        let msg = make_have_msg(pi);
        print_byte_array("have", &msg);
        assert_eq!(msg.len(), 9); // should be 9 bytes long
        assert_eq!(msg.get(3).unwrap(), &5u8);
        assert_eq!(msg.get(4).unwrap(), &4u8);
        assert_eq!(msg.get(8).unwrap(), &4u8);
    }

    #[test]
    fn parse_peer_msg() {
        let resp = [
            0x13, 0x42, 0x69, 0x74, 0x54, 0x6F, 0x72, 0x72, 0x65, 0x6E, 0x74, 0x20, 0x70, 0x72,
            0x6F, 0x74, 0x6F, 0x63, 0x6F, 0x6C, 0x0, 0x0, 0x0, 0x0, 0x0, 0x10, 0x0, 0x5, 0x20,
            0x9C, 0x82, 0x26, 0xB2, 0x99, 0xB3, 0x8, 0xBE, 0xAF, 0x2B, 0x9C, 0xD3, 0xFB, 0x49,
            0x21, 0x2D, 0xBD, 0x13, 0xEC, 0x2D, 0x54, 0x52, 0x32, 0x39, 0x34, 0x30, 0x2D, 0x71,
            0x64, 0x36, 0x37, 0x6F, 0x67, 0x6D, 0x6D,
        ]
        .to_vec();
    }
}
