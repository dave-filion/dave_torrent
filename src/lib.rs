use rand::Rng;
use std::io::{Cursor};

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use lava_torrent::torrent::v1::Torrent;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket, IpAddr, TcpStream};

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
            print!("{:X?}", b);
        } else {
            print!(", {:X?}", b);
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
) -> Vec<u8> {
    let mut buf = ByteBuffer::new();

    // conn id (8 bytes)
    buf.write_bytes(&conn_id_bytes);

    // action (1 for announce, 4 bytes)
    buf.write_u32(1);

    // transaction_id (random 4 bytes)
    let tx_id = rand::thread_rng().gen::<[u8; 4]>();
    print_byte_array("random tx id", &tx_id);
    buf.write_bytes(&tx_id);

    // info hash (20 bytes)
    buf.write_bytes(&info_hash_bytes);

    // peer id (random 20 bytes)
    let peer_id = rand::thread_rng().gen::<[u8; 20]>();
    print_byte_array("random peer id", &peer_id);
    buf.write_bytes(&peer_id);

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
    print_byte_array("Request", &connect_packet);

    // send message to remote udp port
    let _result = sock.send(&connect_packet);

    // TODO: retry logic
    // listen for response
    let mut response_buf = [0; 16];
    let _num_bytes = sock.recv(&mut response_buf).expect("Didnt recieve data");
    print_byte_array("Response", &response_buf);

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
) -> AnnounceResponse {
    // now send announce message and return response
    let announce_packet =
        make_announce_packet(&conn_id_bytes, &info_hash_bytes, torrent_size, port);
    print_byte_array("Announce", &announce_packet);

    let _result = sock.send(&announce_packet);

    // listen for response
    let mut response_buf = [0; 128];
    let _num_bytes = sock.recv(&mut response_buf).unwrap();
    print_byte_array("announce resp", &response_buf);

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
            for k in i..i+4 {
                let b = resp.get(k).unwrap().clone();
                ip_addr[j] = b;
                j+=1;
            }
            // print_byte_array("ipaddr", &ip_addr);

            //get next 2 bytes for port
            let mut port = [0; 2];
            let mut j = 0;
            for k in i+4..i+6 {
                let b = resp.get(k).unwrap().clone();
                port[j] = b;
                j+=1;
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
// handshake: <pstrlen><pstr><reserved><info_hash><peer_id>
//
// pstrlen: string length of <pstr>, as a single raw byte
// pstr: string identifier of the protocol
// reserved: eight (8) reserved bytes. All current implementations use all zeroes.
// peer_id: 20-byte string used as a unique ID for the client.
//
// In version 1.0 of the BitTorrent protocol, pstrlen = 19, and pstr = "BitTorrent protocol".
pub fn connect_to_peer(ip: IpAddr, port: u16) {
    let sock_addr = SocketAddr::new(ip, port);
    println!("connecting to remote socket at addr: {:?}", sock_addr);
    if let Ok(stream) = TcpStream::connect(sock_addr) {
        println!("connected to peer @ {:?}", sock_addr);
    } else {
        println!("Failed to connect to peer");
    }
}

// makes a tcp handshake packet
fn make_handshake(peer_id: [u8; 20]) -> Vec<u8> {
    let buf = ByteBuffer::new();


    buf.to_bytes()
}

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_make_announce_packet() {
        let port = 6969;
        let torrent_size = 1; // 1 so its easy to see in byte form
        let conn_id_bytes = [0x10, 0x38, 0x94, 0xC3, 0x73, 0x6B, 0x76, 0xB2].to_vec();
        let info_hash_bytes = [
            0xAA, 0x16, 0x30, 0x38, 0x78, 0x53, 0x79, 0x81, 0x90, 0x75, 0x43, 0x56, 0x11, 0x51,
            0x73, 0x33, 0x45, 0x89, 0x19, 0xCC,
        ]
            .to_vec();
        let result = make_announce_packet(&conn_id_bytes, &info_hash_bytes, torrent_size, port);

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
        ].to_vec();

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
        let ip = Ipv4Addr::new(66,108,98,51);
        let ip = IpAddr::V4(ip);
        let port = 34264;
        // connect_to_peer(ip, port);

    }
}
