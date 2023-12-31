use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::Mutex;
use std::{sync::atomic::Ordering, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const PORT: u16 = 2000;
const LISTEN_ADDRESS: &str = "0.0.0.0";
const EXISTANCE_BROADCAST_MILLIS: u64 = 1500;
type StdinMsg = [u8; 32];

const LISTEN_SOCKET: (&str, u16) = (LISTEN_ADDRESS, PORT);

static IN_CONNECTION: AtomicBool = AtomicBool::new(false);

static NEW_STDIN: AtomicBool = AtomicBool::new(false);
static STDIN_MSG: Mutex<StdinMsg> = Mutex::new([0; 32]);

/// wait until new message and return it, while marking it as not new
async fn get_new_stdin_msg() -> StdinMsg {
    loop {
        if !NEW_STDIN.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        NEW_STDIN.store(false, Ordering::Relaxed);
        return *STDIN_MSG.lock().unwrap();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut name: StdinMsg = [0; 32];
    println!("Your name: ");
    tokio::io::stdin().read(&mut name).await.unwrap();
    for c in name.iter_mut() {
        if *c == b'\n' {
            *c = b'\0';
        }
    }
    tokio::spawn(broadcast_existance(name));
    let peers = Arc::new(Mutex::new(HashMap::new()));
    tokio::spawn(listen_for_peers(peers.clone(), name));
    tokio::spawn(print_available_peers(peers.clone()));

    tokio::spawn(async move {
        stdin_listener().await.unwrap();
    });
    tokio::spawn(async move {
        tcp_connection_listener().await.unwrap();
    });
    tokio::spawn(async move {
        tcp_connection_creator(peers.clone(), name).await;
    });
    loop {}
}

async fn print_available_peers(peers: Arc<Mutex<HashMap<IpAddr, StdinMsg>>>) {
    let mut interval = tokio::time::interval(Duration::from_millis(1500));
    loop {
        interval.tick().await;
        // only print this info when not in connection
        if IN_CONNECTION.load(Ordering::Relaxed) {
            continue;
        }
        println!("Peers Available:");
        {
            let set = peers.lock().unwrap();
            for (idx, elem) in set.iter().enumerate() {
                print!("  {}: {:?}: ", idx, elem.0);
                use std::io::Write;
                std::io::stdout().write_all(elem.1).unwrap();
                println!();
            }
        }
    }
}

// receives from STDIN and tries to create connections
async fn tcp_connection_creator(peers: Arc<Mutex<HashMap<IpAddr, StdinMsg>>>, my_name: StdinMsg) {
    loop {
        if IN_CONNECTION.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }
        let num = {
            let inp = get_new_stdin_msg().await;
            if IN_CONNECTION.load(Ordering::Relaxed) {
                continue;
            }
            // turn u8's into utf8
            let len = inp
                .iter()
                .position(|&item| item == b'\n')
                .unwrap_or(inp.len());
            let inp = &inp[..len];
            let inp = std::str::from_utf8(inp).unwrap_or("ERRENOUS UTF8");
            let Ok(num) = inp.parse::<u16>() else {
                println!("ERROR: {:?} is not a number", inp);
                continue;
            };
            num
        };
        let (peer, peer_name) = {
            let set = peers.lock().unwrap();

            let Some(peer) = set.iter().skip(num as _).next() else {
                println!("ERROR: no such peer exists!");
                continue;
            };
            (*peer.clone().0, *peer.1)
        };
        let Ok(mut tcp_stream) = tokio::net::TcpStream::connect((peer, PORT)).await else {
            println!("ERROR: Could not connect!");
            continue;
        };
        tcp_stream.write_all(&my_name).await.unwrap();
        spawn_connection_from(tcp_stream, peer_name).await;
    }
}

async fn tcp_connection_listener() -> Result<(), Box<dyn std::error::Error>> {
    let tcp = TcpListener::bind(LISTEN_SOCKET).await?;
    loop {
        let (mut stream, addr) = tcp.accept().await?;
        println!("got connection from: {:?}", addr);
        let mut buf = [0; 32];
        stream.read(&mut buf).await.unwrap();

        spawn_connection_from(stream, buf).await;
    }
}

async fn spawn_connection_from(stream: tokio::net::TcpStream, peer_name: StdinMsg) {
    if IN_CONNECTION.load(Ordering::Relaxed) {
        println!("BUT already in connection!");
        return;
    }
    IN_CONNECTION.store(true, Ordering::Relaxed);
    println!(
        "CONNECTION CREATED WITH: {:?}!",
        stream.peer_addr().unwrap()
    );
    let (read_half, write_half) = stream.into_split();
    let atom = Arc::new(AtomicBool::new(false));
    tokio::spawn(tcp_connection_receiver(read_half, atom.clone(), peer_name));
    tokio::spawn(tcp_connection_writer(write_half, atom));
}

async fn tcp_connection_writer(mut write: tokio::net::tcp::OwnedWriteHalf, done: Arc<AtomicBool>) {
    loop {
        let mut my_buf = [0; 32];
        get_new_stdin_msg().await.clone_into(&mut my_buf);
        if done.load(Ordering::Relaxed) {
            break;
        }
        if &my_buf[0..4] == b"exit" || &my_buf[0..4] == b"quit" {
            println!("MANUALLY QUITING CONNECTION!");
            write.shutdown().await.unwrap();
            break;
        }
        if write.write_all(&my_buf).await.is_err() {
            break;
        }
    }
    if !done.load(Ordering::Relaxed) {
        done.store(true, Ordering::Relaxed);
        IN_CONNECTION.store(false, Ordering::Relaxed);
    }
}

/// read from TcpListener and print to STDOUT
async fn tcp_connection_receiver(
    mut read: tokio::net::tcp::OwnedReadHalf,
    done: Arc<AtomicBool>,
    peer_name: StdinMsg,
) {
    let mut stdout = tokio::io::stdout();
    loop {
        let mut buf = [0; 256];
        let Ok(len) = read.read(&mut buf).await else {
            break;
        };
        if len == 0 {
            if !done.load(Ordering::Relaxed) {
                println!("PEER DISCONNECTED!");
            }
            break;
        }
        if done.load(Ordering::Relaxed) {
            break;
        }
        stdout.write_all(&peer_name).await.unwrap();
        stdout.write_all(b": ").await.unwrap();
        stdout.write_all(&buf).await.unwrap();
    }
    if !done.load(Ordering::Relaxed) {
        done.store(true, Ordering::Relaxed);
        IN_CONNECTION.store(false, Ordering::Relaxed);
    }
}

async fn stdin_listener() -> Result<(), Box<dyn std::error::Error>> {
    println!("Listening on STDIN");

    let mut stdin = tokio::io::stdin();
    loop {
        let mut buf = [0; 32];
        stdin.read(&mut buf).await?;
        buf.clone_into(&mut STDIN_MSG.lock().unwrap());
        NEW_STDIN.store(true, Ordering::Relaxed);
    }
}

async fn broadcast_existance(name: StdinMsg) {
    let udp_sock = tokio::net::UdpSocket::bind(("0.0.0.0", 0)).await.unwrap();
    udp_sock.set_broadcast(true).unwrap();
    println!("broadcasting existance");
    loop {
        udp_sock
            .send_to(&name, ("255.255.255.255", PORT as _))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(EXISTANCE_BROADCAST_MILLIS)).await;
    }
}
async fn listen_for_peers(peers: Arc<Mutex<HashMap<IpAddr, StdinMsg>>>, name: StdinMsg) {
    let udp_sock = tokio::net::UdpSocket::bind(("0.0.0.0", PORT as _))
        .await
        .unwrap();
    let mut buf = [0; 32];
    println!("listening for peers");
    loop {
        let (_size, peer) = udp_sock.recv_from(&mut buf).await.unwrap();
        if name == buf {
            // they have the same name as us
            continue;
        }

        if {
            let mut set = peers.lock().unwrap();
            set.insert(peer.ip(), buf).is_none()
        } {
            // added a new person
            print!("got new peer: {:?}, name:", peer);
            tokio::io::stdout().write_all(&buf).await.unwrap();
            tokio::io::stdout().write_all(b"\n").await.unwrap();
        }
    }
}
