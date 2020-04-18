use rand::{AsByteSliceMut, Rng};
use std::io::{Cursor, Read, Write};

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use std::fmt::Error;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use crate::*;

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
    buf.write_bytes(&key);

    // num_want (4 bytes, -1 is default)
    buf.write_i32(-1);

    // port (2 bytes)
    buf.write_u16(port);

    buf.to_bytes()
}


pub fn perform_announce(
    sock: &UdpSocket,
    conn_id_bytes: &Vec<u8>,
    info_hash_bytes: &Vec<u8>,
    torrent_size: u64,
    port: u16,
    peer_id: &[u8; 20],
    tx_id: &[u8; 4],
) -> Result<AnnounceResponse, Error> {
    // now send announce message and return response
    let announce_packet = make_announce_packet(
        &conn_id_bytes,
        &info_hash_bytes,
        torrent_size,
        port,
        peer_id,
        tx_id,
    );

    let mut attempt = 1;
    let max_attempts = 5;
    loop {

        if attempt > max_attempts {
            println!("max attempts reached. quitting");
            return Err(Error)
        }
        println!("> Perform announce attempt ({}):", attempt);

        print!("Sending announce packet...");
        match sock.send(&announce_packet) {
            Ok(_) => {
                println!("sent!");
            },
            Err(e) => {
                println!("error {:?}", e);
                attempt += 1;
                continue;
            }
        }

        print!("Waiting for announce response...");
        let mut response_buf = [0; 512];
        match sock.recv(&mut response_buf) {
            Ok(bytes_read) => {
                println!("got {} byte response", bytes_read);
                return Ok(response_buf.to_vec().into());
            },
            Err(e) => {
                println!("error {:?}",e);
                attempt += 1;
                continue;
            }
        }
    }


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