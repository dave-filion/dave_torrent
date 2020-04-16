use rand::Rng;
use std::io::{Cursor};

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use lava_torrent::torrent::v1::Torrent;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket, IpAddr};

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